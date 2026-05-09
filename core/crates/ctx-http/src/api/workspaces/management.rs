use super::*;
use crate::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree;
use ctx_workspace_config as workspace_config;

mod attachment_ops;
mod config_ops;
mod file_completions;
mod prompt_config;
mod provider_model_preferences;
mod worktree_bootstrap;

use attachment_ops::*;
use config_ops::*;
pub(in crate::api) use file_completions::workspace_file_completions;
pub(in crate::api) use prompt_config::*;
pub(in crate::api) use provider_model_preferences::*;
pub(in crate::api) use worktree_bootstrap::{
    get_worktree_bootstrap_config, update_worktree_bootstrap_config,
};

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
