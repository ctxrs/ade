use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};

use super::errors::ApiErrorResp;
use super::shared::{load_and_cache_workspace_files, FileCompletionsQuery};
use crate::attachments;
use crate::completions;
use crate::daemon::AppState;
use crate::logs;
use crate::telemetry::TelemetryEvent;
use crate::vcs_hooks;
use crate::workspace_config;
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, Workspace, WorkspaceActiveHeadBatch,
    WorkspaceActiveSnapshot, WorkspaceAttachment, WorkspaceAttachmentKind, Worktree,
};
use ctx_fs::git::assert_git_repo;
use ctx_fs::vcs;

pub(super) async fn get_worktree(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Worktree>, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_worktree(worktree_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    match store
        .get_worktree(worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(wt) => Ok(Json(wt)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub(super) async fn get_worktree_bootstrap_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let worktree_id = WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_worktree(worktree_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let worktree = store
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

#[derive(Debug, Deserialize)]
pub(super) struct CreateWorkspaceReq {
    root_path: String,
    name: Option<String>,
}

pub(super) async fn list_workspaces(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Workspace>>, StatusCode> {
    state
        .global_store()
        .list_workspaces()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub(super) async fn get_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Workspace>, StatusCode> {
    let id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    match state
        .global_store()
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

pub(super) async fn get_workspace_active_snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceActiveSnapshot>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await;
    let snapshot = state
        .workspace_active_snapshot
        .active_snapshot(workspace_id, i64::MAX)
        .await;
    state
        .cache_workspace_active_snapshot(snapshot.clone())
        .await;
    Ok(Json(snapshot))
}

pub(super) async fn get_workspace_active_heads(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceActiveHeadBatch>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await;
    let heads = state
        .workspace_active_snapshot
        .active_heads(workspace_id)
        .await;
    state.cache_workspace_active_heads(heads.clone()).await;
    Ok(Json(heads))
}

pub(super) async fn create_workspace(
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

    let vcs = vcs::driver_for_path(&root_path).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    vcs.assert_repo(&root_path).await.map_err(|e| {
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
        .global_store()
        .create_workspace(name, root_path_str, vcs.kind())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    if let Err(e) = state.store_for_workspace(workspace.id).await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        ));
    }
    state
        .telemetry
        .emit(TelemetryEvent::workspace_registered())
        .await;
    Ok(Json(workspace))
}

pub(super) async fn delete_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let worktrees = match state.store_for_workspace(id).await {
        Ok(store) => store.list_worktrees(id).await.unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    state.cleanup_workspace(id).await;
    state
        .global_store()
        .delete_workspace_indexes(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .global_store()
        .delete_workspace(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.stores.evict_workspace(id).await;
    for worktree in &worktrees {
        if let Err(err) = vcs_hooks::cleanup_worktree_hooks(
            &state.data_root,
            id,
            worktree.id,
            Some(StdPath::new(&worktree.root_path)),
            worktree.vcs_kind.clone(),
        )
        .await
        {
            tracing::warn!(
                workspace_id = %id.0,
                worktree_id = %worktree.id.0,
                "failed to remove vcs hooks: {err:#}"
            );
        }
    }
    if let Err(err) = vcs_hooks::cleanup_workspace_hooks(&state.data_root, id).await {
        tracing::warn!(
            workspace_id = %id.0,
            "failed to remove vcs hooks: {err:#}"
        );
    }
    let workspace_db_dir = state
        .data_root
        .join("db")
        .join("workspaces")
        .join(id.0.to_string());
    let _ = tokio::fs::remove_dir_all(workspace_db_dir).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub(super) struct SyncWorkspaceAttachmentsReq {
    #[serde(default)]
    refresh: Option<bool>,
}

pub(super) async fn list_workspace_attachments(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<WorkspaceAttachment>>, StatusCode> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_workspace(ws_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    store
        .list_workspace_attachments(ws_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub(super) async fn sync_workspace_attachments(
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
        .global_store()
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
    let attachments =
        attachments::sync_workspace_attachments(Arc::clone(&state), &workspace, refresh)
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
pub(super) struct AgentSystemPromptConfigResponse {
    config_path: String,
    default_append: String,
    configured_append: Option<String>,
    effective_append: Option<String>,
    source: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateAgentSystemPromptConfigReq {
    #[serde(default)]
    system_prompt_append: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct SubagentSystemPromptConfigResponse {
    config_path: String,
    default_append: String,
    configured_append: Option<String>,
    effective_append: Option<String>,
    source: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateSubagentSystemPromptConfigReq {
    #[serde(default)]
    system_prompt_append: Option<String>,
}

pub(super) async fn get_agent_system_prompt(
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
        .global_store()
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

pub(super) async fn update_agent_system_prompt(
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
        .global_store()
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

pub(super) async fn get_subagent_system_prompt(
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
        .global_store()
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

pub(super) async fn update_subagent_system_prompt(
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
        .global_store()
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
pub(super) struct CreateWorkspaceAttachmentReq {
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

pub(super) async fn create_workspace_attachment(
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
        .global_store()
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

    let attachments = attachments::sync_workspace_attachments(Arc::clone(&state), &workspace, true)
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
pub(super) struct DeleteWorkspaceAttachmentReq {
    kind: WorkspaceAttachmentKind,
    name: String,
}

pub(super) async fn delete_workspace_attachment(
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
        .global_store()
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

    let attachments =
        attachments::sync_workspace_attachments(Arc::clone(&state), &workspace, false)
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

pub(super) async fn workspace_file_completions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FileCompletionsQuery>,
) -> Result<Json<Vec<String>>, StatusCode> {
    const DEFAULT_LIMIT: u32 = 20;
    const MAX_LIMIT: u32 = 200;
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10);

    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let ws = state
        .global_store()
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
        let mut cache = state.workspace_file_completions_cache.lock().await;
        if let Some(entry) = cache.get_mut(&ws_id) {
            entry.touch();
            if now.duration_since(entry.value.cached_at) <= CACHE_TTL {
                entry.value.files.clone()
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
