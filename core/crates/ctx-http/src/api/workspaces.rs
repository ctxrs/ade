use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};

mod active;
mod attachments;
mod context;
mod harness_container;
mod management;
mod registry;
mod worktrees;

pub(super) use active::{get_workspace_active_heads, get_workspace_active_snapshot};
pub(super) use attachments::{list_workspace_attachments, sync_workspace_attachments};
use context::*;
pub(super) use harness_container::{
    ensure_workspace_harness_container, get_workspace_harness_container,
    stop_workspace_harness_container,
};
pub(in crate::api) use management::*;
pub(super) use registry::{create_workspace, delete_workspace, get_workspace, list_workspaces};
pub(super) use worktrees::{get_worktree, get_worktree_bootstrap_logs};

use super::errors::ApiErrorResp;
use super::shared::map_effective_execution_settings_error;
use ctx_daemon::daemon::workspaces::{WorkspaceHydrationError, WorkspaceHydrationErrorKind};
use ctx_daemon::daemon::{
    CreateWorkspaceAttachmentRouteRequest, DeleteWorkspaceAttachmentRouteRequest,
    SyncWorkspaceAttachmentsRouteRequest, UpdateWorkspaceExecutionConfigRequest,
    UpdateWorkspacePrimaryBranchRequest, WorkspaceActiveHeadBatchRouteResponse,
    WorkspaceActiveSnapshotRouteResponse, WorkspaceAttachmentRouteResponse,
    WorkspaceConfigUpdateResult, WorkspaceExecutionConfigSnapshot,
    WorkspaceHarnessContainerStatusRouteResponse, WorkspacePrimaryBranchSnapshot,
    WorkspaceRouteError, WorkspaceRouteErrorKind, WorkspaceRouteResponse, WorkspacesHandle,
    WorktreeRouteResponse,
};
use ctx_observability::logs;

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

#[cfg(test)]
mod tests;

fn workspace_route_api_error(error: WorkspaceRouteError) -> (StatusCode, Json<ApiErrorResp>) {
    let status = workspace_route_status(&error);
    (
        status,
        Json(ApiErrorResp {
            error: logs::redact_sensitive(error.message()),
        }),
    )
}

fn workspace_route_status(error: &WorkspaceRouteError) -> StatusCode {
    match error.kind() {
        WorkspaceRouteErrorKind::NotFound => StatusCode::NOT_FOUND,
        WorkspaceRouteErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        WorkspaceRouteErrorKind::Forbidden => StatusCode::FORBIDDEN,
        WorkspaceRouteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
