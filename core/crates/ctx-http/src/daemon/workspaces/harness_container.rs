use std::sync::Arc;

use ctx_core::ids::WorkspaceId;
use ctx_settings_service::EffectiveExecutionSettingsError;
use ctx_workspace_container::WorkspaceContainerStatus;

use crate::daemon::{execution_effective, DaemonState};

#[derive(Debug)]
pub(crate) enum WorkspaceHarnessContainerError {
    NotFound,
    Internal(anyhow::Error),
    ExecutionSettings(EffectiveExecutionSettingsError),
    Ensure(anyhow::Error),
}

pub(crate) async fn workspace_harness_container_status(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Result<Option<WorkspaceContainerStatus>, WorkspaceHarnessContainerError> {
    ensure_workspace_exists(state, workspace_id).await?;
    state
        .execution
        .harness
        .container_status(workspace_id)
        .await
        .map_err(WorkspaceHarnessContainerError::Internal)
}

pub(crate) async fn stop_workspace_harness_container(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Result<(), WorkspaceHarnessContainerError> {
    ensure_workspace_exists(state, workspace_id).await?;
    let stopped = state
        .execution
        .harness
        .stop_container(workspace_id)
        .await
        .map_err(WorkspaceHarnessContainerError::Internal)?;
    if stopped {
        Ok(())
    } else {
        Err(WorkspaceHarnessContainerError::NotFound)
    }
}

pub(crate) async fn ensure_workspace_harness_container(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Result<(), WorkspaceHarnessContainerError> {
    let workspace = ensure_workspace_exists(state, workspace_id).await?;
    let execution_settings =
        execution_effective::effective_execution_settings_classified(state.as_ref(), workspace_id)
            .await
            .map_err(WorkspaceHarnessContainerError::ExecutionSettings)?;
    state
        .execution
        .harness
        .ensure_workspace_container(&workspace, &execution_settings, &state.core.daemon_url)
        .await
        .map_err(WorkspaceHarnessContainerError::Ensure)
}

async fn ensure_workspace_exists(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Result<ctx_core::models::Workspace, WorkspaceHarnessContainerError> {
    state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(WorkspaceHarnessContainerError::Internal)?
        .ok_or(WorkspaceHarnessContainerError::NotFound)
}
