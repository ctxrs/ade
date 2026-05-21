use ctx_observability::telemetry::TelemetryEvent;
use ctx_repo_onboarding_service::prepare_workspace_registration;
use ctx_workspace_config as workspace_config;
use serde::Deserialize;

use crate::daemon::{WorkspaceStoreAccessError, WorkspacesHandle};

use super::WorkspaceRouteResponse;

mod management_config;
mod prompt_and_model;

pub use ctx_route_contracts::workspaces::{
    CreateWorkspaceRequest, UpdateWorkspacePrimaryBranchRequest, WorkspaceConfigUpdateResult,
    WorkspacePrimaryBranchSnapshot, WorkspaceRouteError, WorkspaceRouteErrorKind,
};
pub use management_config::{
    UpdateWorkspaceMergeQueueConfigRequest, UpdateWorktreeBootstrapConfigRequest,
    WorkspaceMergeQueueConfigRouteResponse, WorkspaceWorktreeBootstrapConfigRouteResponse,
};
pub(in crate::daemon::workspaces) use prompt_and_model::{
    parse_workspace_route_id, provider_model_preference_error, workspace_store_error,
};
pub use prompt_and_model::{
    AgentSystemPromptConfigRouteResponse, SubagentSystemPromptConfigRouteResponse,
    UpdateAgentSystemPromptConfigRouteRequest, UpdateSubagentSystemPromptConfigRouteRequest,
    UpdateWorkspaceProviderModelPreferenceRouteRequest, WorkspacePromptConfigRouteParams,
    WorkspaceProviderModelPreferenceRouteParams, WorkspaceProviderModelPreferenceRouteResponse,
};

#[derive(Debug, Deserialize)]
pub struct UpdateWorkspaceExecutionConfigRequest {
    pub environment: String,
    #[serde(default)]
    pub network_mode: Option<String>,
    #[serde(default)]
    pub allowlist: Option<Vec<String>>,
}

pub type WorkspaceExecutionConfigSnapshot = workspace_config::ExecutionConfigSnapshot;

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
