use ctx_observability::telemetry::TelemetryEvent;
use ctx_workspace_config as workspace_config;
use ctx_workspace_services::workspace_registration::prepare_workspace_registration;
use serde::{Deserialize, Serialize};

use crate::daemon::{WorkspaceStoreAccessError, WorkspacesHandle};

use super::WorkspaceRouteResponse;

mod management_config;
mod prompt_and_model;

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
pub struct CreateWorkspaceRequest {
    pub root_path: String,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWorkspacePrimaryBranchRequest {
    pub primary_branch: String,
}

#[derive(Debug, Serialize)]
pub struct WorkspacePrimaryBranchSnapshot {
    pub primary_branch: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWorkspaceExecutionConfigRequest {
    pub environment: String,
    #[serde(default)]
    pub network_mode: Option<String>,
    #[serde(default)]
    pub allowlist: Option<Vec<String>>,
}

pub type WorkspaceExecutionConfigSnapshot = workspace_config::ExecutionConfigSnapshot;

#[derive(Debug, Serialize)]
pub struct WorkspaceConfigUpdateResult {
    pub ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceRouteErrorKind {
    NotFound,
    BadRequest,
    Forbidden,
    InsufficientStorage,
    Internal,
}

#[derive(Debug, Clone)]
pub struct WorkspaceRouteError {
    kind: WorkspaceRouteErrorKind,
    message: String,
}

impl WorkspaceRouteError {
    pub(in crate::daemon::workspaces) fn new(
        kind: WorkspaceRouteErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(in crate::daemon::workspaces) fn not_found(message: impl Into<String>) -> Self {
        Self::new(WorkspaceRouteErrorKind::NotFound, message)
    }

    pub(in crate::daemon::workspaces) fn bad_request(error: impl std::fmt::Display) -> Self {
        Self::new(WorkspaceRouteErrorKind::BadRequest, error.to_string())
    }

    pub(in crate::daemon::workspaces) fn forbidden(error: impl std::fmt::Display) -> Self {
        Self::new(WorkspaceRouteErrorKind::Forbidden, error.to_string())
    }

    pub(in crate::daemon::workspaces) fn insufficient_storage(
        error: impl std::fmt::Display,
    ) -> Self {
        Self::new(
            WorkspaceRouteErrorKind::InsufficientStorage,
            error.to_string(),
        )
    }

    pub(in crate::daemon::workspaces) fn internal(error: impl std::fmt::Display) -> Self {
        Self::new(WorkspaceRouteErrorKind::Internal, error.to_string())
    }

    pub(in crate::daemon::workspaces) fn from_request_or_policy_error(
        error: anyhow::Error,
    ) -> Self {
        if ctx_settings_service::is_execution_policy_denial(&error) {
            Self::forbidden(error)
        } else {
            Self::bad_request(error)
        }
    }

    pub(in crate::daemon::workspaces) fn from_workspace_store(
        error: WorkspaceStoreAccessError,
    ) -> Self {
        match error {
            WorkspaceStoreAccessError::NotFound => Self::not_found("workspace not found"),
            WorkspaceStoreAccessError::Unavailable(error) => Self::internal(error),
        }
    }

    pub fn kind(&self) -> WorkspaceRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
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
            .map_err(WorkspaceRouteError::from_workspace_store)?;
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
