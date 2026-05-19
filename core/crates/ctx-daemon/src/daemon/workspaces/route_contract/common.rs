use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::Workspace;
use ctx_observability::logs;
use ctx_settings_service::EffectiveExecutionSettingsError;

use crate::daemon::RouteFileDownloadError;

use super::super::{
    FileCompletionsError, FileCompletionsErrorKind, WorkspaceDeleteError,
    WorkspaceHarnessContainerError, WorkspaceHydrationError, WorkspaceRouteError, WorkspacesHandle,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WorkspaceRouteParams {
    workspace_id: String,
}

impl WorkspaceRouteParams {
    pub fn new(workspace_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
        }
    }

    pub(super) fn parse_workspace_id(&self) -> Result<WorkspaceId, WorkspaceRouteError> {
        uuid::Uuid::parse_str(&self.workspace_id)
            .map(WorkspaceId)
            .map_err(|_| WorkspaceRouteError::bad_request("invalid workspace id"))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WorktreeRouteParams {
    worktree_id: String,
}

impl WorktreeRouteParams {
    pub fn new(worktree_id: impl Into<String>) -> Self {
        Self {
            worktree_id: worktree_id.into(),
        }
    }

    pub(super) fn parse_worktree_id(&self) -> Result<WorktreeId, WorkspaceRouteError> {
        uuid::Uuid::parse_str(&self.worktree_id)
            .map(WorktreeId)
            .map_err(|_| WorkspaceRouteError::bad_request("invalid worktree id"))
    }
}

pub(super) fn workspace_hydration_route_error(
    error: WorkspaceHydrationError,
) -> WorkspaceRouteError {
    match error {
        WorkspaceHydrationError::NotFound => WorkspaceRouteError::not_found("workspace not found"),
        WorkspaceHydrationError::Load(error) => {
            WorkspaceRouteError::internal(logs::redact_sensitive(&error.to_string()))
        }
    }
}

pub(super) fn workspace_delete_route_error(error: WorkspaceDeleteError) -> WorkspaceRouteError {
    match error {
        WorkspaceDeleteError::NotFound => WorkspaceRouteError::not_found("workspace not found"),
        WorkspaceDeleteError::Internal => {
            WorkspaceRouteError::internal("failed to delete workspace")
        }
    }
}

pub(super) fn route_file_download_error(error: RouteFileDownloadError) -> WorkspaceRouteError {
    match error {
        RouteFileDownloadError::NotFound => WorkspaceRouteError::not_found("file not found"),
        RouteFileDownloadError::Internal => {
            WorkspaceRouteError::internal("failed to read route file")
        }
    }
}

pub(super) fn workspace_harness_container_status_error(
    error: WorkspaceHarnessContainerError,
) -> WorkspaceRouteError {
    match error {
        WorkspaceHarnessContainerError::NotFound => {
            WorkspaceRouteError::not_found("workspace not found")
        }
        WorkspaceHarnessContainerError::Internal(_)
        | WorkspaceHarnessContainerError::ExecutionSettings(_)
        | WorkspaceHarnessContainerError::Ensure(_) => {
            WorkspaceRouteError::internal("workspace harness container request failed")
        }
    }
}

pub(super) fn effective_execution_settings_route_error(
    error: EffectiveExecutionSettingsError,
) -> WorkspaceRouteError {
    match error {
        EffectiveExecutionSettingsError::InvalidWorkspaceOverride(error) => {
            let message = logs::redact_sensitive(&error.to_string());
            if ctx_settings_service::is_execution_policy_denial(&error) {
                WorkspaceRouteError::forbidden(message)
            } else {
                WorkspaceRouteError::bad_request(message)
            }
        }
        EffectiveExecutionSettingsError::Internal(error) => {
            WorkspaceRouteError::internal(logs::redact_sensitive(&error.to_string()))
        }
    }
}

pub(super) fn workspace_harness_container_ensure_error(
    error: WorkspaceHarnessContainerError,
) -> WorkspaceRouteError {
    match error {
        WorkspaceHarnessContainerError::NotFound => {
            WorkspaceRouteError::not_found("workspace not found")
        }
        WorkspaceHarnessContainerError::Internal(error) => {
            WorkspaceRouteError::internal(logs::redact_sensitive(&error.to_string()))
        }
        WorkspaceHarnessContainerError::ExecutionSettings(error) => {
            effective_execution_settings_route_error(error)
        }
        WorkspaceHarnessContainerError::Ensure(error) => {
            WorkspaceRouteError::bad_request(logs::redact_sensitive(&error.to_string()))
        }
    }
}

pub(super) fn file_completions_route_error(error: FileCompletionsError) -> WorkspaceRouteError {
    match error.kind() {
        FileCompletionsErrorKind::NotFound => WorkspaceRouteError::not_found(error.message()),
        FileCompletionsErrorKind::Forbidden => WorkspaceRouteError::forbidden(error.message()),
        FileCompletionsErrorKind::InsufficientStorage => {
            WorkspaceRouteError::insufficient_storage(error.message())
        }
        FileCompletionsErrorKind::Internal => {
            tracing::warn!(error = error.message(), "file completions request failed");
            WorkspaceRouteError::internal(error.message())
        }
    }
}

pub(super) async fn require_workspace_for_route(
    handle: &WorkspacesHandle,
    workspace_id: WorkspaceId,
) -> Result<Workspace, WorkspaceRouteError> {
    handle
        .get_workspace(workspace_id)
        .await
        .map_err(WorkspaceRouteError::internal)?
        .ok_or_else(|| WorkspaceRouteError::not_found("workspace not found"))
}
