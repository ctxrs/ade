use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};

mod management;

pub(in crate::api) use management::*;

use super::errors::ApiErrorResp;
use super::shared::{
    load_and_cache_workspace_files, map_effective_execution_settings_error,
    store_for_existing_workspace_status, FileCompletionsQuery,
};
use crate::attachments;
use crate::completions;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::workspace_runtime::HarnessContainerStatus;
use crate::logs;
use crate::telemetry::TelemetryEvent;
use crate::vcs_hooks;
use crate::workspace_config;
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, VcsKind, Workspace, WorkspaceActiveHeadBatch,
    WorkspaceActiveSnapshot, WorkspaceAttachment, WorkspaceAttachmentKind, Worktree,
};
use ctx_fs::git::{assert_git_repo, git_default_branch};
use ctx_fs::vcs;

#[derive(Debug, Deserialize)]
pub(super) struct UpdateMergeQueueConfigReq {
    enabled: bool,
    #[serde(default)]
    target_branch: Option<String>,
    #[serde(default)]
    verify_command: Option<String>,
    #[serde(default)]
    push_on_success: Option<bool>,
    #[serde(default)]
    push_remote: Option<String>,
    #[serde(default)]
    push_branch: Option<String>,
}
#[derive(Debug, Serialize)]
pub(super) struct UpdateWorkspaceConfigResp {
    ok: bool,
}
#[derive(Debug, Serialize)]
pub(super) struct WorkspaceMergeQueueConfigResp {
    enabled: bool,
    target_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    verify_command: Option<String>,
    push_on_success: bool,
    push_remote: String,
    push_branch: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateWorkspacePrimaryBranchReq {
    primary_branch: String,
}

#[derive(Debug, Serialize)]
pub(super) struct WorkspacePrimaryBranchResp {
    primary_branch: String,
}

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

async fn detect_workspace_primary_branch(
    vcs_kind: VcsKind,
    root_path: &StdPath,
    driver: &dyn vcs::VcsDriver,
) -> anyhow::Result<String> {
    match vcs_kind {
        VcsKind::Git => {
            let branch = git_default_branch(root_path)
                .await?
                .ok_or_else(|| anyhow::anyhow!("unable to detect default git branch"))?;
            let trimmed = branch.trim().to_string();
            if trimmed.is_empty() {
                anyhow::bail!("detected default git branch is empty");
            }
            Ok(trimmed)
        }
        VcsKind::Jj => {
            driver
                .rev_parse_ref(root_path, "main")
                .await
                .context("resolving jj primary bookmark `main`")?;
            Ok("main".to_string())
        }
        _ => anyhow::bail!("primary branch detection is only supported for git and jj workspaces"),
    }
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
                .telemetry
                .emit(TelemetryEvent::workspace_opened())
                .await;
            Ok(Json(ws))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub(super) async fn get_workspace_harness_container(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Option<HarnessContainerStatus>>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if workspace.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let status = state
        .execution
        .harness
        .container_status(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(status))
}

pub(super) async fn stop_workspace_harness_container(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if workspace.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let stopped = state
        .execution
        .harness
        .stop_container(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if stopped {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub(super) async fn ensure_workspace_harness_container(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
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

    let execution_settings =
        execution_effective::effective_execution_settings_classified(state.as_ref(), workspace_id)
            .await
            .map_err(map_effective_execution_settings_error)?;

    state
        .execution
        .harness
        .ensure_workspace_container(&workspace, &execution_settings, &state.core.daemon_url)
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?;

    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn get_workspace_active_snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceActiveSnapshot>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await
        .map_err(|err| err.status_code())?;
    crate::merge_queue::activate_workspace_merge_queue(&state, workspace_id).await;
    let snapshot = state
        .workspaces
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
        .await
        .map_err(|err| err.status_code())?;
    crate::merge_queue::activate_workspace_merge_queue(&state, workspace_id).await;
    let heads = state
        .workspaces
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

    let primary_branch = detect_workspace_primary_branch(vcs.kind(), &root_path, vcs.as_ref())
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
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
    let store = state.store_for_workspace(workspace.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    workspace_config::update_primary_branch(&store, &primary_branch)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    state
        .telemetry
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
    let workspace = state
        .global_store()
        .get_workspace(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let worktrees = match state.store_for_workspace(id).await {
        Ok(store) => store.list_worktrees(id).await.unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    state.core.stores.begin_workspace_delete(id).await;
    let delete_result = async {
        for worktree in &worktrees {
            if let Err(err) = vcs_hooks::cleanup_worktree_hooks(&state, &workspace, worktree).await
            {
                tracing::warn!(
                    workspace_id = %id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove vcs hooks: {err:#}"
                );
            }
        }
        state.cleanup_workspace(id).await;
        state.core.stores.evict_workspace_and_wait_closed(id).await;
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
        Ok::<(), StatusCode>(())
    }
    .await;
    state.core.stores.finish_workspace_delete(id).await;
    delete_result?;
    if let Err(err) = vcs_hooks::cleanup_workspace_hooks(&state.core.data_root, id).await {
        tracing::warn!(
            workspace_id = %id.0,
            "failed to remove vcs hooks: {err:#}"
        );
    }
    let workspace_db_dir = state
        .core
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
    let store = store_for_existing_workspace_status(&state, ws_id).await?;
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
