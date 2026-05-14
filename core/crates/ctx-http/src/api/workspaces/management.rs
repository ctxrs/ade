use super::*;
use ctx_workspace_config as workspace_config;

mod attachment_ops;
mod attachment_routes;
mod config_ops;
mod file_completions;
mod prompt_config;
mod provider_model_preferences;
mod worktree_bootstrap;

use attachment_ops::*;
pub(in crate::api) use attachment_routes::{
    create_workspace_attachment, delete_workspace_attachment,
};
pub(super) use attachment_routes::{CreateWorkspaceAttachmentReq, DeleteWorkspaceAttachmentReq};
use config_ops::*;
pub(in crate::api) use file_completions::workspace_file_completions;
pub(in crate::api) use prompt_config::*;
pub(in crate::api) use provider_model_preferences::*;
pub(in crate::api) use worktree_bootstrap::{
    get_worktree_bootstrap_config, update_worktree_bootstrap_config,
};

pub(in crate::api) async fn get_workspace_primary_branch(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<WorkspacePrimaryBranchResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&workspaces, &id).await?;
    load_workspace_primary_branch(&workspaces, &ctx)
        .await
        .map(Json)
}

pub(in crate::api) async fn update_workspace_primary_branch(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorkspacePrimaryBranchReq>,
) -> Result<Json<WorkspacePrimaryBranchResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&workspaces, &id).await?;
    update_workspace_primary_branch_config(&workspaces, &ctx, req)
        .await
        .map(Json)
}

pub(in crate::api) async fn update_merge_queue_config(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMergeQueueConfigReq>,
) -> Result<Json<UpdateWorkspaceConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&workspaces, &id).await?;
    update_workspace_merge_queue_config(&workspaces, &ctx, req)
        .await
        .map(Json)
}

pub(in crate::api) async fn get_merge_queue_config(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceMergeQueueConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&workspaces, &id).await?;
    load_workspace_merge_queue_config(&workspaces, &ctx)
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
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceExecutionConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&workspaces, &id).await?;
    load_workspace_execution_config(&workspaces, &ctx)
        .await
        .map(Json)
}

pub(in crate::api) async fn update_execution_config(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Json(req): Json<UpdateExecutionConfigReq>,
) -> Result<Json<UpdateWorkspaceConfigResp>, (StatusCode, Json<ApiErrorResp>)> {
    let ctx = require_workspace_ctx(&workspaces, &id).await?;
    update_workspace_execution_config(&workspaces, &ctx, req)
        .await
        .map(Json)
}
