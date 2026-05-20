use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;

mod active;
mod attachments;
mod harness_container;
mod management;
mod registry;
mod worktrees;

pub(super) use active::{get_workspace_active_heads, get_workspace_active_snapshot};
pub(super) use attachments::{list_workspace_attachments, sync_workspace_attachments};
pub(super) use harness_container::{
    ensure_workspace_harness_container, get_workspace_harness_container,
    stop_workspace_harness_container,
};
pub(in crate::api) use management::*;
pub(super) use registry::{create_workspace, delete_workspace, get_workspace, list_workspaces};
pub(super) use worktrees::{get_worktree, get_worktree_bootstrap_logs};

use super::errors::ApiErrorResp;
use ctx_daemon::daemon::{
    AgentSystemPromptConfigRouteResponse, SubagentSystemPromptConfigRouteResponse,
    UpdateAgentSystemPromptConfigRouteRequest, UpdateSubagentSystemPromptConfigRouteRequest,
    UpdateWorkspaceExecutionConfigRequest, UpdateWorkspaceMergeQueueConfigRequest,
    UpdateWorkspaceProviderModelPreferenceRouteRequest, UpdateWorktreeBootstrapConfigRequest,
    WorkspaceExecutionConfigSnapshot, WorkspaceHarnessContainerStatusRouteResponse,
    WorkspaceMergeQueueConfigRouteResponse, WorkspacePromptConfigRouteParams,
    WorkspaceProviderModelPreferenceRouteParams, WorkspaceProviderModelPreferenceRouteResponse,
    WorkspaceWorktreeBootstrapConfigRouteResponse, WorkspacesHandle,
};
use ctx_observability::logs;
use ctx_route_contracts::workspaces::{
    CreateWorkspaceAttachmentRouteRequest, DeleteWorkspaceAttachmentRouteRequest,
    SyncWorkspaceAttachmentsRouteRequest, UpdateWorkspacePrimaryBranchRequest,
    WorkspaceActiveHeadBatchRouteResponse, WorkspaceActiveSnapshotRouteResponse,
    WorkspaceAttachmentRouteResponse, WorkspaceConfigUpdateResult, WorkspacePrimaryBranchSnapshot,
    WorkspaceRouteError, WorkspaceRouteErrorKind, WorkspaceRouteParams, WorkspaceRouteResponse,
    WorktreeRouteParams, WorktreeRouteResponse,
};

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
        WorkspaceRouteErrorKind::InsufficientStorage => StatusCode::INSUFFICIENT_STORAGE,
        WorkspaceRouteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
