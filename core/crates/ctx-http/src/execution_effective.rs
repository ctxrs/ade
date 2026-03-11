use ctx_core::ids::WorkspaceId;
use ctx_core::models::ExecutionEnvironment as SessionExecutionEnvironment;

use crate::daemon::AppState;
use crate::installs::InstallTarget;
use crate::settings::ExecutionSettings;
use crate::{settings, workspace_config};

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
        workspace_config::apply_execution_settings_override(&mut effective, &ov);
    }
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
    if matches!(settings.mode, crate::settings::ExecutionMode::Container) {
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

pub fn apply_execution_environment(
    settings: &mut ExecutionSettings,
    execution_environment: SessionExecutionEnvironment,
) {
    match execution_environment {
        SessionExecutionEnvironment::Host => {
            settings.mode = crate::settings::ExecutionMode::Host;
        }
        SessionExecutionEnvironment::ContainerHostMounted => {
            settings.mode = crate::settings::ExecutionMode::Container;
            settings.container.mount_mode = crate::settings::ContainerMountMode::HostMounted;
        }
        SessionExecutionEnvironment::ContainerDiskIsolated => {
            settings.mode = crate::settings::ExecutionMode::Container;
            settings.container.mount_mode = crate::settings::ContainerMountMode::DiskIsolated;
        }
    }
}

pub async fn effective_execution_settings_for_environment(
    state: &AppState,
    workspace_id: WorkspaceId,
    execution_environment: SessionExecutionEnvironment,
) -> anyhow::Result<ExecutionSettings> {
    let mut effective = effective_execution_settings(state, workspace_id).await?;
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

#[cfg(test)]
mod tests {
    use super::{effective_install_target, install_target_for_settings};

    use std::collections::HashMap;

    use ctx_core::ids::WorkspaceId;
    use ctx_store::StoreManager;

    use crate::daemon::AppState;
    use crate::installs::InstallTarget;
    use crate::settings::{ExecutionMode, ExecutionSettings};

    #[test]
    fn install_target_for_settings_matches_execution_mode() {
        let host = ExecutionSettings {
            mode: ExecutionMode::Host,
            ..ExecutionSettings::default()
        };
        let container = ExecutionSettings {
            mode: ExecutionMode::Container,
            ..ExecutionSettings::default()
        };

        assert_eq!(install_target_for_settings(&host), InstallTarget::Host);
        assert_eq!(
            install_target_for_settings(&container),
            InstallTarget::Container
        );
    }

    #[tokio::test]
    async fn effective_install_target_errors_for_missing_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stores = StoreManager::open(temp.path()).await.expect("open stores");
        let state = AppState::new(
            temp.path().to_path_buf(),
            stores,
            HashMap::new(),
            "http://127.0.0.1:4310".to_string(),
            None,
        );

        let err = effective_install_target(&state, WorkspaceId::new())
            .await
            .expect_err("missing workspace should fail");
        let message = format!("{err:#}");
        assert!(message.contains("workspace"));
        assert!(message.contains("not found"));
    }
}
