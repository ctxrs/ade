use std::sync::Arc;

use tokio::sync::broadcast;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::Workspace;
use ctx_execution_runtime::{
    route_contract::{
        LinuxSandboxActivationMode, LinuxSandboxRuntimeError, LinuxSandboxRuntimeOperation,
        LinuxSandboxRuntimePrepareResult, LinuxSandboxRuntimeStatus, StartExecutionLaunchError,
        StartExecutionLaunchRequest,
    },
    ExecutionLaunchSnapshot, ExecutionLaunchStreamEvent, ExecutionSetupJobKind,
    RuntimePrewarmScope, StartupPrewarmSnapshot,
};
use ctx_linux_sandbox_runtime::{
    linux_sandbox_runtime_status as runtime_linux_sandbox_runtime_status,
    prepare_linux_sandbox_runtime as runtime_prepare_linux_sandbox_runtime,
    stage_linux_sandbox_runtime_downloads as runtime_stage_linux_sandbox_runtime_downloads,
};
use ctx_observability::logs;
use ctx_settings_model::{ExecutionMode, ExecutionSettings};
use ctx_settings_service::EffectiveExecutionSettingsError;

use crate::daemon::{maintenance, settings, DaemonState, ExecutionHandle};

fn classify_effective_execution_settings_error(
    error: EffectiveExecutionSettingsError,
) -> StartExecutionLaunchError {
    match error {
        EffectiveExecutionSettingsError::InvalidWorkspaceOverride(error) => {
            StartExecutionLaunchError::InvalidWorkspaceExecutionSettings {
                policy_denial: ctx_settings_service::is_execution_policy_denial(&error),
                message: logs::redact_sensitive(&error.to_string()),
            }
        }
        EffectiveExecutionSettingsError::Internal(error) => StartExecutionLaunchError::Internal {
            message: logs::redact_sensitive(&error.to_string()),
        },
    }
}

fn parse_workspace_id(raw_workspace_id: &str) -> Result<WorkspaceId, StartExecutionLaunchError> {
    uuid::Uuid::parse_str(raw_workspace_id.trim())
        .map(WorkspaceId)
        .map_err(|_| StartExecutionLaunchError::InvalidWorkspaceId)
}

fn linux_sandbox_runtime_error(
    operation: LinuxSandboxRuntimeOperation,
    error: anyhow::Error,
) -> LinuxSandboxRuntimeError {
    let operation_name = match operation {
        LinuxSandboxRuntimeOperation::Status => "linux_sandbox_runtime_status",
        LinuxSandboxRuntimeOperation::Stage => "linux_sandbox_runtime_stage",
        LinuxSandboxRuntimeOperation::Prepare => "linux_sandbox_runtime_prepare",
    };
    tracing::warn!(
        target: "linux_sandbox",
        error = %logs::redact_sensitive(&error.to_string()),
        "{operation_name} error"
    );
    LinuxSandboxRuntimeError::Runtime {
        operation,
        message: operation.user_message().to_string(),
    }
}

fn linux_sandbox_prepare_drain_error(
    error: maintenance::MaintenanceDrainError,
) -> LinuxSandboxRuntimeError {
    match error {
        maintenance::MaintenanceDrainError::AlreadyActive => {
            LinuxSandboxRuntimeError::PrepareAlreadyActive
        }
        maintenance::MaintenanceDrainError::ActivityUnavailable(error) => {
            tracing::warn!(
                target: "linux_sandbox",
                error = %logs::redact_sensitive(&error.to_string()),
                "linux_sandbox_runtime_prepare activity gate error"
            );
            LinuxSandboxRuntimeError::PrepareActivityUnavailable {
                message: LinuxSandboxRuntimeOperation::Prepare
                    .user_message()
                    .to_string(),
            }
        }
        maintenance::MaintenanceDrainError::SandboxWorkActive => {
            LinuxSandboxRuntimeError::PrepareSandboxWorkActive
        }
    }
}

pub async fn launch_status(
    state: &Arc<DaemonState>,
    job_id: &str,
) -> Option<ExecutionLaunchSnapshot> {
    state.execution.setup.launch_status(job_id).await
}

pub async fn subscribe_launch(
    state: &Arc<DaemonState>,
    job_id: &str,
) -> Option<(
    ExecutionLaunchSnapshot,
    broadcast::Receiver<ExecutionLaunchStreamEvent>,
)> {
    state.execution.setup.subscribe_launch(job_id).await
}

pub async fn start_workspace_launch(
    state: &Arc<DaemonState>,
    workspace: Workspace,
    execution_settings: ExecutionSettings,
) -> ExecutionLaunchSnapshot {
    state
        .execution
        .setup
        .start_workspace_launch(workspace, execution_settings, state.core.daemon_url.clone())
        .await
}

pub async fn start_runtime_prewarm(
    state: &Arc<DaemonState>,
    execution_settings: ExecutionSettings,
    prewarm_scope: RuntimePrewarmScope,
) -> ExecutionLaunchSnapshot {
    state
        .execution
        .setup
        .start_runtime_prewarm(execution_settings, prewarm_scope)
        .await
}

pub async fn startup_status(state: &Arc<DaemonState>) -> StartupPrewarmSnapshot {
    state.execution.setup.startup_status().await
}

pub async fn start_execution_launch_for_request(
    state: &Arc<DaemonState>,
    request: StartExecutionLaunchRequest,
) -> Result<ExecutionLaunchSnapshot, StartExecutionLaunchError> {
    maintenance::reject_new_execution_during_maintenance(state.as_ref())
        .await
        .map_err(|error| StartExecutionLaunchError::MaintenanceActive {
            message: logs::redact_sensitive(&error.to_string()),
        })?;

    let kind = request
        .kind
        .unwrap_or(ExecutionSetupJobKind::WorkspaceLaunch);
    match kind {
        ExecutionSetupJobKind::WorkspaceLaunch => {
            let raw_workspace_id = request
                .workspace_id
                .as_deref()
                .ok_or(StartExecutionLaunchError::MissingWorkspaceId)?;
            let workspace_id = parse_workspace_id(raw_workspace_id)?;
            let workspace = state
                .global_store()
                .get_workspace(workspace_id)
                .await
                .map_err(|error| StartExecutionLaunchError::Internal {
                    message: logs::redact_sensitive(&error.to_string()),
                })?
                .ok_or(StartExecutionLaunchError::WorkspaceNotFound)?;
            let execution_settings =
                crate::daemon::execution_effective::effective_execution_settings_classified(
                    state.as_ref(),
                    workspace_id,
                )
                .await
                .map_err(classify_effective_execution_settings_error)?;
            Ok(start_workspace_launch(state, workspace, execution_settings).await)
        }
        ExecutionSetupJobKind::StartupPrewarm => {
            let settings = settings::load_settings(state.as_ref())
                .await
                .map_err(|error| StartExecutionLaunchError::Internal {
                    message: logs::redact_sensitive(&error.to_string()),
                })?;
            let mut execution_settings = settings.execution.unwrap_or_default();
            execution_settings.mode = ExecutionMode::Sandbox;
            Ok(start_runtime_prewarm(state, execution_settings, request.prewarm_scope).await)
        }
    }
}

pub async fn linux_sandbox_runtime_status(
    state: &Arc<DaemonState>,
) -> Result<LinuxSandboxRuntimeStatus, LinuxSandboxRuntimeError> {
    runtime_linux_sandbox_runtime_status(&state.core.data_root)
        .await
        .map_err(|error| linux_sandbox_runtime_error(LinuxSandboxRuntimeOperation::Status, error))
}

pub async fn stage_linux_sandbox_runtime(
    state: &Arc<DaemonState>,
) -> Result<LinuxSandboxRuntimeStatus, LinuxSandboxRuntimeError> {
    runtime_stage_linux_sandbox_runtime_downloads(&state.core.data_root, None)
        .await
        .map_err(|error| linux_sandbox_runtime_error(LinuxSandboxRuntimeOperation::Stage, error))
}

pub async fn prepare_linux_sandbox_runtime(
    state: &Arc<DaemonState>,
    activation_mode: Option<LinuxSandboxActivationMode>,
    sudo_password: Option<&str>,
) -> Result<LinuxSandboxRuntimePrepareResult, LinuxSandboxRuntimeError> {
    let drain_permit = maintenance::acquire_linux_sandbox_prepare_drain(state)
        .await
        .map_err(linux_sandbox_prepare_drain_error)?;
    let result = match runtime_prepare_linux_sandbox_runtime(
        &state.core.data_root,
        activation_mode.unwrap_or(LinuxSandboxActivationMode::Local),
        sudo_password,
        None,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let _ = drain_permit.release().await;
            return Err(linux_sandbox_runtime_error(
                LinuxSandboxRuntimeOperation::Prepare,
                error,
            ));
        }
    };
    let _ = drain_permit.release().await;
    Ok(result)
}

impl ExecutionHandle {
    pub async fn launch_status(&self, job_id: &str) -> Option<ExecutionLaunchSnapshot> {
        launch_status(&self.state, job_id).await
    }

    pub async fn subscribe_launch(
        &self,
        job_id: &str,
    ) -> Option<(
        ExecutionLaunchSnapshot,
        broadcast::Receiver<ExecutionLaunchStreamEvent>,
    )> {
        subscribe_launch(&self.state, job_id).await
    }

    pub async fn start_workspace_launch(
        &self,
        workspace: Workspace,
        execution_settings: ExecutionSettings,
    ) -> ExecutionLaunchSnapshot {
        start_workspace_launch(&self.state, workspace, execution_settings).await
    }

    pub async fn start_runtime_prewarm(
        &self,
        execution_settings: ExecutionSettings,
        prewarm_scope: RuntimePrewarmScope,
    ) -> ExecutionLaunchSnapshot {
        start_runtime_prewarm(&self.state, execution_settings, prewarm_scope).await
    }

    pub async fn startup_status(&self) -> StartupPrewarmSnapshot {
        startup_status(&self.state).await
    }

    pub async fn start_execution_launch_for_request(
        &self,
        request: StartExecutionLaunchRequest,
    ) -> Result<ExecutionLaunchSnapshot, StartExecutionLaunchError> {
        start_execution_launch_for_request(&self.state, request).await
    }

    pub async fn linux_sandbox_runtime_status(
        &self,
    ) -> Result<LinuxSandboxRuntimeStatus, LinuxSandboxRuntimeError> {
        linux_sandbox_runtime_status(&self.state).await
    }

    pub async fn stage_linux_sandbox_runtime(
        &self,
    ) -> Result<LinuxSandboxRuntimeStatus, LinuxSandboxRuntimeError> {
        stage_linux_sandbox_runtime(&self.state).await
    }

    pub async fn prepare_linux_sandbox_runtime(
        &self,
        activation_mode: Option<LinuxSandboxActivationMode>,
        sudo_password: Option<&str>,
    ) -> Result<LinuxSandboxRuntimePrepareResult, LinuxSandboxRuntimeError> {
        prepare_linux_sandbox_runtime(&self.state, activation_mode, sudo_password).await
    }
}
