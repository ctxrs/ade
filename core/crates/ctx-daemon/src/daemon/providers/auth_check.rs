use std::sync::Arc;

use ctx_core::ids::WorkspaceId;
pub use ctx_provider_runtime::provider_auth_check::ProviderAuthCheckSnapshot;
use ctx_provider_runtime::provider_auth_check::{
    ProviderAuthCheckServiceError, ProviderWorkspaceAuthenticationError,
};
pub use ctx_provider_runtime::provider_launch::config_snapshot::ProviderLaunchConfigError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::daemon::providers::install_target_for_workspace;
use crate::daemon::{DaemonState, ProvidersHandle};

mod workspace;

use workspace::load_workspace;

#[derive(Debug)]
pub enum ProviderAuthCheckError {
    WorkspaceLoad,
    WorkspaceNotFound,
    ExecutionSettings(anyhow::Error),
    ProviderLaunchConfig(ProviderLaunchConfigError),
    Verify(String),
}

#[derive(Debug, Deserialize)]
pub struct AuthenticateProviderForWorkspaceRouteBody {
    #[serde(default)]
    pub method_id: Option<String>,
}

pub struct AuthenticateProviderForWorkspaceRouteRequest {
    pub workspace_id: String,
    pub provider_id: String,
    pub method_id: Option<String>,
}

pub struct VerifyProviderForWorkspaceRouteRequest {
    pub workspace_id: String,
    pub provider_id: String,
}

#[derive(Debug, Serialize)]
pub struct ProviderAuthCheckRouteResponse {
    pub provider_id: String,
    pub workspace_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl From<ProviderAuthCheckSnapshot> for ProviderAuthCheckRouteResponse {
    fn from(value: ProviderAuthCheckSnapshot) -> Self {
        Self {
            provider_id: value.provider_id,
            workspace_id: value.workspace_id,
            status: value.status,
            auth_required: value.auth_required,
            checked_at: value.checked_at,
            message: value.message,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ProviderAuthCheckRouteErrorStatus {
    BadRequest,
    NotFound,
    InternalServerError,
}

#[derive(Debug)]
pub struct ProviderAuthCheckRouteError {
    status: ProviderAuthCheckRouteErrorStatus,
    body: Value,
}

impl ProviderAuthCheckRouteError {
    pub fn status(&self) -> ProviderAuthCheckRouteErrorStatus {
        self.status
    }

    pub fn body(&self) -> &Value {
        &self.body
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: ProviderAuthCheckRouteErrorStatus::BadRequest,
            body: serde_json::json!({
                "error": message.into(),
            }),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: ProviderAuthCheckRouteErrorStatus::NotFound,
            body: serde_json::json!({
                "error": message.into(),
            }),
        }
    }

    fn internal_server_error(message: impl Into<String>) -> Self {
        Self {
            status: ProviderAuthCheckRouteErrorStatus::InternalServerError,
            body: serde_json::json!({
                "error": message.into(),
            }),
        }
    }
}

pub async fn authenticate_provider_for_workspace(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
    method_id: Option<String>,
) -> Result<ProviderAuthCheckSnapshot, ProviderAuthCheckError> {
    let workspace = load_workspace(state, workspace_id).await?;
    let install_target = install_target_for_workspace(state, workspace.id)
        .await
        .map_err(ProviderAuthCheckError::ExecutionSettings)?;
    ctx_provider_runtime::provider_auth_check::authenticate_provider_for_workspace_runtime(
        state.as_ref(),
        &workspace,
        workspace_id,
        provider_id,
        install_target,
        method_id,
    )
    .await
    .map_err(|error| match error {
        ProviderWorkspaceAuthenticationError::Verify(error) => {
            ProviderAuthCheckError::Verify(error)
        }
    })
}

impl ProvidersHandle {
    pub async fn authenticate_provider_for_workspace_for_route(
        &self,
        request: AuthenticateProviderForWorkspaceRouteRequest,
    ) -> Result<ProviderAuthCheckRouteResponse, ProviderAuthCheckRouteError> {
        let workspace_id = parse_workspace_id_for_auth_route(&request.workspace_id)?;
        authenticate_provider_for_workspace(
            &self.state,
            workspace_id,
            &request.provider_id,
            request.method_id,
        )
        .await
        .map(ProviderAuthCheckRouteResponse::from)
        .map_err(provider_auth_check_route_error)
    }

    pub async fn verify_provider_for_workspace_for_route(
        &self,
        request: VerifyProviderForWorkspaceRouteRequest,
    ) -> Result<ProviderAuthCheckRouteResponse, ProviderAuthCheckRouteError> {
        let workspace_id = parse_workspace_id_for_auth_route(&request.workspace_id)?;
        verify_provider_for_workspace(&self.state, workspace_id, &request.provider_id)
            .await
            .map(ProviderAuthCheckRouteResponse::from)
            .map_err(provider_auth_check_route_error)
    }
}

fn parse_workspace_id_for_auth_route(
    raw: &str,
) -> Result<WorkspaceId, ProviderAuthCheckRouteError> {
    uuid::Uuid::parse_str(raw)
        .map(WorkspaceId)
        .map_err(|_| ProviderAuthCheckRouteError::bad_request("invalid workspace id"))
}

fn provider_auth_check_route_error(error: ProviderAuthCheckError) -> ProviderAuthCheckRouteError {
    match error {
        ProviderAuthCheckError::WorkspaceLoad => {
            ProviderAuthCheckRouteError::internal_server_error("failed to load workspace")
        }
        ProviderAuthCheckError::WorkspaceNotFound => {
            ProviderAuthCheckRouteError::not_found("workspace not found")
        }
        ProviderAuthCheckError::ExecutionSettings(error) => {
            ProviderAuthCheckRouteError::internal_server_error(format!(
                "failed to load workspace execution settings: {error:#}"
            ))
        }
        ProviderAuthCheckError::ProviderLaunchConfig(error) => {
            provider_launch_config_route_error(error)
        }
        ProviderAuthCheckError::Verify(error) => ProviderAuthCheckRouteError::bad_request(error),
    }
}

fn provider_launch_config_route_error(
    error: ProviderLaunchConfigError,
) -> ProviderAuthCheckRouteError {
    match error {
        ProviderLaunchConfigError::UnsupportedProvider { provider_id } => {
            ProviderAuthCheckRouteError::bad_request(format!(
                "unsupported provider id: {provider_id}"
            ))
        }
    }
}

pub async fn verify_provider_for_workspace(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
) -> Result<ProviderAuthCheckSnapshot, ProviderAuthCheckError> {
    let workspace = load_workspace(state, workspace_id).await?;
    let install_target = install_target_for_workspace(state, workspace.id)
        .await
        .map_err(ProviderAuthCheckError::ExecutionSettings)?;
    ctx_provider_runtime::provider_auth_check::verify_provider_for_workspace_runtime(
        state.as_ref(),
        &workspace,
        workspace_id,
        provider_id,
        install_target,
    )
    .await
    .map_err(|error| match error {
        ProviderAuthCheckServiceError::ProviderLaunchConfig(error) => {
            ProviderAuthCheckError::ProviderLaunchConfig(error)
        }
        ProviderAuthCheckServiceError::Verify(error) => ProviderAuthCheckError::Verify(error),
    })
}
