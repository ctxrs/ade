use ctx_core::ids::WorkspaceId;
use ctx_workspace_config as workspace_config;

use crate::daemon::AppState;
use crate::execution_policy::HostExecutionPolicy;
use crate::settings;
use crate::settings::ExecutionSettings;
use ctx_provider_install::install_state::InstallTarget;

mod environment;
mod override_settings;

pub use environment::{
    effective_execution_settings_for_environment, effective_install_target_for_environment,
};
pub(crate) use override_settings::{
    apply_workspace_execution_settings_override, validate_workspace_execution_settings_override,
};

#[derive(Debug)]
pub enum EffectiveExecutionSettingsError {
    InvalidWorkspaceOverride(anyhow::Error),
    Internal(anyhow::Error),
}

impl EffectiveExecutionSettingsError {
    pub fn into_inner(self) -> anyhow::Error {
        match self {
            Self::InvalidWorkspaceOverride(err) | Self::Internal(err) => err,
        }
    }
}

pub async fn effective_execution_settings_classified(
    state: &AppState,
    workspace_id: WorkspaceId,
) -> Result<ExecutionSettings, EffectiveExecutionSettingsError> {
    let settings_data = settings::load_settings(state.global_store())
        .await
        .map_err(EffectiveExecutionSettingsError::Internal)?;
    let mut effective = settings_data.execution.clone().unwrap_or_default();
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(EffectiveExecutionSettingsError::Internal)?;
    if let Some(ov) = workspace_config::load_execution_settings_override(&store)
        .await
        .map_err(EffectiveExecutionSettingsError::InvalidWorkspaceOverride)?
    {
        apply_workspace_execution_settings_override(&mut effective, &ov)
            .map_err(EffectiveExecutionSettingsError::InvalidWorkspaceOverride)?;
    }
    HostExecutionPolicy::current()
        .and_then(|policy| policy.validate_execution_settings(&effective))
        .map_err(EffectiveExecutionSettingsError::InvalidWorkspaceOverride)?;
    Ok(effective)
}

/// Compute effective execution settings for a workspace, combining daemon defaults with any
/// workspace runtime override.
pub async fn effective_execution_settings(
    state: &AppState,
    workspace_id: WorkspaceId,
) -> anyhow::Result<ExecutionSettings> {
    effective_execution_settings_classified(state, workspace_id)
        .await
        .map_err(EffectiveExecutionSettingsError::into_inner)
}

pub fn install_target_for_settings(settings: &ExecutionSettings) -> InstallTarget {
    if matches!(settings.mode, crate::settings::ExecutionMode::Sandbox) {
        InstallTarget::Container
    } else {
        InstallTarget::Host
    }
}

pub async fn effective_install_target(
    state: &AppState,
    workspace_id: WorkspaceId,
) -> anyhow::Result<InstallTarget> {
    let effective = effective_execution_settings(state, workspace_id).await?;
    Ok(install_target_for_settings(&effective))
}

#[cfg(test)]
mod execution_effective_test;
