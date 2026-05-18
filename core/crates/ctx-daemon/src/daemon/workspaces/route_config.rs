use std::path::Path;

use ctx_core::ids::WorkspaceId;
use ctx_observability::telemetry::TelemetryEvent;
use ctx_settings_model::{ContainerNetworkMode, ExecutionMode, ExecutionSettings};
use ctx_workspace_config as workspace_config;
use ctx_workspace_services::workspace_registration::{
    prepare_workspace_registration, validate_workspace_primary_branch,
};
use serde::{Deserialize, Serialize};

use crate::daemon::{settings, WorkspaceStoreAccessError, WorkspacesHandle};

use super::WorkspaceRouteResponse;

mod management_config;
mod prompt_and_model;

pub use management_config::{
    UpdateWorkspaceMergeQueueConfigRequest, UpdateWorktreeBootstrapConfigRequest,
    WorkspaceMergeQueueConfigRouteResponse, WorkspaceWorktreeBootstrapConfigRouteResponse,
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

#[derive(Debug, Serialize)]
pub struct WorkspaceExecutionConfigSnapshot {
    pub source: String,
    pub environment: String,
    pub network_mode: Option<String>,
    pub allowlist: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceConfigUpdateResult {
    pub ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceRouteErrorKind {
    NotFound,
    BadRequest,
    Forbidden,
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

    pub async fn workspace_primary_branch_for_request(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspacePrimaryBranchSnapshot, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?;
        let primary_branch = workspace_config::load_primary_branch(&store)
            .await
            .map_err(WorkspaceRouteError::internal)?
            .ok_or_else(|| {
                WorkspaceRouteError::not_found("workspace primary branch is not configured")
            })?;
        Ok(WorkspacePrimaryBranchSnapshot { primary_branch })
    }

    pub async fn update_workspace_primary_branch_for_request(
        &self,
        workspace_id: WorkspaceId,
        req: UpdateWorkspacePrimaryBranchRequest,
    ) -> Result<WorkspacePrimaryBranchSnapshot, WorkspaceRouteError> {
        let workspace = self
            .state
            .global_store()
            .get_workspace(workspace_id)
            .await
            .map_err(WorkspaceRouteError::internal)?
            .ok_or_else(|| WorkspaceRouteError::not_found("workspace not found"))?;
        let primary_branch =
            validate_workspace_primary_branch(Path::new(&workspace.root_path), &req.primary_branch)
                .await
                .map_err(|error| WorkspaceRouteError::bad_request(error.message()))?;
        let store = self
            .existing_workspace_store(workspace.id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?;
        workspace_config::update_primary_branch(&store, &primary_branch)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        let worktrees = store
            .list_worktrees(workspace.id)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        for worktree in worktrees {
            if let Err(error) = self.refresh_worktree_vcs_snapshot(&worktree, true).await {
                tracing::warn!(
                    workspace_id = %workspace.id.0,
                    worktree_id = %worktree.id.0,
                    "failed to refresh worktree vcs after primary branch update: {error:#}"
                );
            }
        }
        Ok(WorkspacePrimaryBranchSnapshot { primary_branch })
    }

    pub async fn workspace_execution_config_for_request(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceExecutionConfigSnapshot, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?;
        let settings = settings::load_settings(self.state.as_ref())
            .await
            .map_err(WorkspaceRouteError::internal)?;
        let mut effective = settings.execution.clone().unwrap_or_default();
        let mut source = "daemon_default".to_string();
        match workspace_config::load_execution_settings_override(&store).await {
            Ok(Some(override_config)) => {
                ctx_settings_service::apply_workspace_execution_settings_override(
                    &mut effective,
                    &override_config,
                )
                .map_err(WorkspaceRouteError::from_request_or_policy_error)?;
                source = "workspace".to_string();
            }
            Ok(None) => {}
            Err(error) if workspace_config::is_workspace_runtime_settings_parse_error(&error) => {
                return Err(WorkspaceRouteError::bad_request(error));
            }
            Err(error) => return Err(WorkspaceRouteError::internal(error)),
        }
        Ok(project_workspace_execution_config(source, &effective))
    }

    pub async fn update_workspace_execution_config_for_request(
        &self,
        workspace_id: WorkspaceId,
        req: UpdateWorkspaceExecutionConfigRequest,
    ) -> Result<WorkspaceConfigUpdateResult, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(WorkspaceRouteError::from_workspace_store)?;
        let environment = parse_execution_environment_for_request(
            req.environment.trim(),
            self.sandbox_runtime_available_for_execution_config(),
        )?;
        let network_mode = parse_execution_network_mode_for_request(req.network_mode.as_deref())?;
        let allowlist = req.allowlist.map(normalize_execution_allowlist);
        let settings = settings::load_settings(self.state.as_ref())
            .await
            .map_err(WorkspaceRouteError::internal)?;
        let effective = settings.execution.clone().unwrap_or_default();
        let requested_override = build_workspace_execution_config_override(
            environment,
            network_mode.clone(),
            allowlist.clone(),
        );
        ctx_settings_service::validate_workspace_execution_settings_override(
            &effective,
            &requested_override,
        )
        .map_err(WorkspaceRouteError::from_request_or_policy_error)?;
        workspace_config::update_execution_config(
            &store,
            workspace_config::ExecutionConfigUpdate {
                environment,
                network_mode,
                allowlist,
                image: None,
            },
        )
        .await
        .map_err(WorkspaceRouteError::bad_request)?;
        Ok(WorkspaceConfigUpdateResult { ok: true })
    }

    fn sandbox_runtime_available_for_execution_config(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.shared_vm_container_runtime_available()
        }
        #[cfg(not(target_os = "macos"))]
        {
            true
        }
    }
}

fn project_workspace_execution_config(
    source: String,
    effective: &ExecutionSettings,
) -> WorkspaceExecutionConfigSnapshot {
    let environment = match effective.mode {
        ExecutionMode::Host => "host",
        ExecutionMode::Sandbox => "sandbox",
    }
    .to_string();
    let network_mode = match effective.container.network_mode {
        ContainerNetworkMode::LlmOnly => "llm_only",
        ContainerNetworkMode::Allowlist => "allowlist",
        ContainerNetworkMode::All => "all",
    }
    .to_string();

    WorkspaceExecutionConfigSnapshot {
        source,
        environment,
        network_mode: Some(network_mode),
        allowlist: Some(effective.container.allowlist.clone()),
    }
}

fn parse_execution_environment_for_request(
    environment: &str,
    sandbox_runtime_available: bool,
) -> Result<workspace_config::ExecutionEnvironment, WorkspaceRouteError> {
    match environment {
        "host" => Ok(workspace_config::ExecutionEnvironment::Host),
        "sandbox" => {
            if !sandbox_runtime_available {
                return Err(WorkspaceRouteError::bad_request(
                    "AVF sandbox is unavailable on this macOS host. Install or launch through the desktop app so the AVF helper/runtime is present, then try again.",
                ));
            }
            Ok(workspace_config::ExecutionEnvironment::Sandbox)
        }
        _ => Err(WorkspaceRouteError::bad_request(
            "invalid environment (expected host|sandbox)",
        )),
    }
}

fn parse_execution_network_mode_for_request(
    network_mode: Option<&str>,
) -> Result<Option<ContainerNetworkMode>, WorkspaceRouteError> {
    match network_mode.map(str::trim) {
        None | Some("") => Ok(None),
        Some("llm_only") => Ok(Some(ContainerNetworkMode::LlmOnly)),
        Some("allowlist") => Ok(Some(ContainerNetworkMode::Allowlist)),
        Some("all") => Ok(Some(ContainerNetworkMode::All)),
        _ => Err(WorkspaceRouteError::bad_request(
            "invalid network_mode (expected llm_only|allowlist|all)",
        )),
    }
}

fn normalize_execution_allowlist(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn build_workspace_execution_config_override(
    environment: workspace_config::ExecutionEnvironment,
    network_mode: Option<ContainerNetworkMode>,
    allowlist: Option<Vec<String>>,
) -> workspace_config::ExecutionSettingsOverride {
    workspace_config::ExecutionSettingsOverride {
        mode: Some(match environment {
            workspace_config::ExecutionEnvironment::Host => ExecutionMode::Host,
            workspace_config::ExecutionEnvironment::Sandbox => ExecutionMode::Sandbox,
        }),
        container: workspace_config::ContainerExecutionSettingsOverride {
            network_mode,
            allowlist,
            image: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_execution_environment_rejects_unavailable_sandbox_runtime() {
        let error = parse_execution_environment_for_request("sandbox", false)
            .expect_err("sandbox should require available runtime");
        assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert!(
            error.message().contains("AVF sandbox is unavailable"),
            "unexpected error: {}",
            error.message()
        );
    }

    #[test]
    fn parse_execution_environment_accepts_host_without_sandbox_runtime() {
        let environment = parse_execution_environment_for_request("host", false)
            .expect("host mode should not require sandbox runtime");
        assert_eq!(environment, workspace_config::ExecutionEnvironment::Host);
    }

    #[test]
    fn parse_execution_network_mode_rejects_unknown_values() {
        let error = parse_execution_network_mode_for_request(Some("public"))
            .expect_err("unknown network mode should be rejected");
        assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert_eq!(
            error.message(),
            "invalid network_mode (expected llm_only|allowlist|all)"
        );
    }
}
