use super::*;
use crate::git_status::emit_worktree_vcs_snapshot_for_worktree;
use ctx_workspace_config as workspace_config;

mod attachment_ops;
mod config_ops;
mod prompt_config;
mod provider_model_preferences;

use attachment_ops::*;
use config_ops::*;
pub(in crate::api) use prompt_config::*;
pub(in crate::api) use provider_model_preferences::*;

pub(in crate::api) async fn get_workspace_primary_branch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspacePrimaryBranchResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&state, &id).await?;
    load_workspace_primary_branch(&ctx.store).await.map(Json)
}

pub(in crate::api) async fn update_workspace_primary_branch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorkspacePrimaryBranchReq>,
) -> Result<Json<WorkspacePrimaryBranchResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&state, &id).await?;
    update_workspace_primary_branch_config(&state, &ctx, req)
        .await
        .map(Json)
}

pub(in crate::api) async fn update_merge_queue_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMergeQueueConfigReq>,
) -> Result<Json<UpdateWorkspaceConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&state, &id).await?;
    update_workspace_merge_queue_config(&state, &ctx, req)
        .await
        .map(Json)
}

pub(in crate::api) async fn get_merge_queue_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceMergeQueueConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&state, &id).await?;
    load_workspace_merge_queue_config(&ctx.store)
        .await
        .map(Json)
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct UpdateWorktreeBootstrapReq {
    #[serde(default)]
    setup_command: Option<String>,
    #[serde(default)]
    timeout_sec: Option<u64>,
    #[serde(default)]
    wait_for_completion: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct WorkspaceWorktreeBootstrapConfigResp {
    #[serde(skip_serializing_if = "Option::is_none")]
    setup_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait_for_completion: Option<bool>,
}

pub(in crate::api) async fn get_worktree_bootstrap_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceWorktreeBootstrapConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&state, &id).await?;
    let cfg = workspace_config::load_worktree_bootstrap_config(&ctx.store)
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;

    Ok(Json(WorkspaceWorktreeBootstrapConfigResp {
        setup_command: cfg.as_ref().and_then(|value| value.setup_command.clone()),
        timeout_sec: cfg.as_ref().and_then(|value| value.timeout_sec),
        wait_for_completion: cfg.as_ref().and_then(|value| value.wait_for_completion),
    }))
}

pub(in crate::api) async fn update_worktree_bootstrap_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorktreeBootstrapReq>,
) -> Result<Json<UpdateWorkspaceConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&state, &id).await?;
    workspace_config::update_worktree_bootstrap_config(
        &ctx.store,
        workspace_config::WorktreeBootstrapConfigUpdate {
            setup_command: req.setup_command,
            timeout_sec: req.timeout_sec,
            wait_for_completion: req.wait_for_completion,
        },
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

    Ok(Json(UpdateWorkspaceConfigResp { ok: true }))
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct UpdateExecutionConfigReq {
    environment: String,
    #[serde(default)]
    network_mode: Option<String>,
    #[serde(default)]
    allowlist: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct WorkspaceExecutionConfigResp {
    source: String,               // "workspace" | "daemon_default"
    environment: String,          // "host" | "sandbox"
    network_mode: Option<String>, // "llm_only" | "allowlist" | "all"
    allowlist: Option<Vec<String>>,
}

pub(in crate::api) async fn get_execution_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceExecutionConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&state, &id).await?;
    load_workspace_execution_config(&state, &ctx)
        .await
        .map(Json)
}

pub(in crate::api) async fn update_execution_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateExecutionConfigReq>,
) -> Result<Json<UpdateWorkspaceConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&state, &id).await?;
    update_workspace_execution_config(&state, &ctx, req)
        .await
        .map(Json)
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct CreateWorkspaceAttachmentReq {
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

pub(in crate::api) async fn create_workspace_attachment(
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
    let ctx = require_workspace_ctx(&state, &id).await?;
    create_and_sync_workspace_attachment(&state, &ctx, req)
        .await
        .map(Json)
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct DeleteWorkspaceAttachmentReq {
    kind: WorkspaceAttachmentKind,
    name: String,
}

pub(in crate::api) async fn delete_workspace_attachment(
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
    let ctx = require_workspace_ctx(&state, &id).await?;
    delete_and_sync_workspace_attachment(&state, &ctx, req)
        .await
        .map(Json)
}

pub(in crate::api) async fn workspace_file_completions(
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
        let mut cache = state
            .workspaces
            .workspace_file_completions_cache
            .lock()
            .await;
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
