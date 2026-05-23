use ctx_observability::telemetry::TelemetryEvent;
use ctx_repo_onboarding_service::prepare_workspace_registration;
use ctx_route_contracts::workspaces::WorkspaceRouteResponse;
use ctx_workspace_config as workspace_config;

use crate::daemon::{WorkspaceStoreAccessError, WorkspacesHandle};

mod management_config;
mod prompt_and_model;

pub use ctx_route_contracts::workspaces::{
    CreateWorkspaceRequest, UpdateWorkspacePrimaryBranchRequest, WorkspaceConfigUpdateResult,
    WorkspacePrimaryBranchSnapshot, WorkspaceRouteError,
};
pub(in crate::daemon::workspaces) use ctx_route_contracts::workspaces::{
    UpdateWorkspaceMergeQueueConfigRequest, WorkspaceMergeQueueConfigRouteResponse,
};
pub(in crate::daemon::workspaces) use management_config::{
    merge_queue_config_route_response, merge_queue_config_update,
    workspace_execution_config_route_snapshot, worktree_bootstrap_config_route_response,
    worktree_bootstrap_config_update,
};
pub(in crate::daemon::workspaces) use prompt_and_model::{
    agent_system_prompt_config_route_response, provider_model_preference_error,
    provider_model_preference_route_response, subagent_system_prompt_config_route_response,
    workspace_store_error,
};

pub(in crate::daemon::workspaces) fn request_or_policy_route_error(
    error: anyhow::Error,
) -> WorkspaceRouteError {
    if ctx_settings_service::is_execution_policy_denial(&error) {
        WorkspaceRouteError::forbidden(error)
    } else {
        WorkspaceRouteError::bad_request(error)
    }
}

pub(in crate::daemon::workspaces) fn workspace_store_route_error(
    error: WorkspaceStoreAccessError,
) -> WorkspaceRouteError {
    match error {
        WorkspaceStoreAccessError::NotFound => {
            WorkspaceRouteError::not_found("workspace not found")
        }
        WorkspaceStoreAccessError::Unavailable(error) => WorkspaceRouteError::internal(error),
    }
}

impl WorkspacesHandle {
    pub async fn create_workspace_for_request(
        &self,
        req: CreateWorkspaceRequest,
    ) -> Result<WorkspaceRouteResponse, WorkspaceRouteError> {
        let candidate = prepare_workspace_registration(&req.root_path)
            .await
            .map_err(|error| WorkspaceRouteError::bad_request(error.message()))?;
        let root_path = candidate.root_path.to_string_lossy().to_string();
        let name = req.name.unwrap_or(candidate.default_name);
        let workspace = self
            .state
            .global_store()
            .create_workspace(name, root_path, candidate.vcs_kind)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        let store = self
            .existing_workspace_store(workspace.id)
            .await
            .map_err(workspace_store_route_error)?;
        workspace_config::update_primary_branch(&store, &candidate.primary_branch)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        self.state
            .telemetry
            .telemetry
            .emit(TelemetryEvent::workspace_registered())
            .await;
        Ok(workspace.into())
    }
}
