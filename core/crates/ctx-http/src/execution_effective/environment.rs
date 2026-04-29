use ctx_core::ids::WorkspaceId;
use ctx_core::models::ExecutionEnvironment as SessionExecutionEnvironment;
use ctx_provider_install::install_state::InstallTarget;

use crate::daemon::AppState;
use crate::execution_policy::{ExecutionPolicyDenied, HostExecutionPolicy};
use crate::settings::{ExecutionMode, ExecutionSettings};

use super::{effective_execution_settings, install_target_for_settings};

pub fn apply_execution_environment(
    settings: &mut ExecutionSettings,
    execution_environment: SessionExecutionEnvironment,
) {
    match execution_environment {
        SessionExecutionEnvironment::Host => {
            settings.mode = crate::settings::ExecutionMode::Host;
        }
        SessionExecutionEnvironment::Sandbox => {
            settings.mode = crate::settings::ExecutionMode::Sandbox;
            settings.container.mount_mode = crate::settings::ContainerMountMode::DiskIsolated;
        }
    }
}

pub(crate) fn validate_execution_environment_against_settings(
    settings: &ExecutionSettings,
    execution_environment: SessionExecutionEnvironment,
) -> anyhow::Result<()> {
    HostExecutionPolicy::current()?.validate_execution_environment(execution_environment)?;
    if matches!(settings.mode, ExecutionMode::Sandbox)
        && matches!(execution_environment, SessionExecutionEnvironment::Host)
    {
        return Err(ExecutionPolicyDenied::new(
            "session execution environment host is not allowed when effective daemon execution mode is sandbox"
        )
        .into());
    }
    Ok(())
}

pub async fn effective_execution_settings_for_environment(
    state: &AppState,
    workspace_id: WorkspaceId,
    execution_environment: SessionExecutionEnvironment,
) -> anyhow::Result<ExecutionSettings> {
    let mut effective = effective_execution_settings(state, workspace_id).await?;
    validate_execution_environment_against_settings(&effective, execution_environment)?;
    apply_execution_environment(&mut effective, execution_environment);
    Ok(effective)
}

pub async fn effective_install_target_for_environment(
    state: &AppState,
    workspace_id: WorkspaceId,
    execution_environment: SessionExecutionEnvironment,
) -> anyhow::Result<InstallTarget> {
    let effective =
        effective_execution_settings_for_environment(state, workspace_id, execution_environment)
            .await?;
    Ok(install_target_for_settings(&effective))
}
