use ctx_core::ids::WorkspaceId;
use ctx_core::models::Workspace;
use ctx_route_contracts::workspaces::{
    WorkspaceHarnessContainerMountModeRouteValue, WorkspaceHarnessContainerNetworkModeRouteValue,
    WorkspaceHarnessContainerStatusRouteResponse, WorkspaceRouteParams,
};
use ctx_sandbox_contract::{ContainerMountMode, ContainerNetworkMode};
use ctx_settings_service::EffectiveExecutionSettingsError;
use ctx_workspace_container::WorkspaceContainerStatus;

use super::super::{WorkspaceHarnessContainerError, WorkspaceRouteError};
use super::common::{
    workspace_harness_container_ensure_error, workspace_harness_container_status_error,
};
use crate::daemon::WorkspaceHarnessContainerHandle;

impl WorkspaceHarnessContainerHandle {
    pub async fn workspace_harness_container_status_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceHarnessContainerStatusRouteResponse>, WorkspaceHarnessContainerError>
    {
        workspace_harness_container_status(self, workspace_id)
            .await
            .map(|status| status.map(workspace_harness_container_status_route_response))
    }

    pub async fn workspace_harness_container_status_for_route_params(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<Option<WorkspaceHarnessContainerStatusRouteResponse>, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.workspace_harness_container_status_for_route(workspace_id)
            .await
            .map_err(workspace_harness_container_status_error)
    }

    pub async fn stop_workspace_harness_container_for_route(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<(), WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        stop_workspace_harness_container(self, workspace_id)
            .await
            .map_err(workspace_harness_container_status_error)
    }

    pub async fn ensure_workspace_harness_container_for_route(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<(), WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        ensure_workspace_harness_container(self, workspace_id)
            .await
            .map_err(workspace_harness_container_ensure_error)
    }
}

async fn workspace_harness_container_status(
    handle: &WorkspaceHarnessContainerHandle,
    workspace_id: WorkspaceId,
) -> Result<Option<WorkspaceContainerStatus>, WorkspaceHarnessContainerError> {
    ensure_workspace_exists(handle, workspace_id).await?;
    handle
        .harness()
        .container_status(workspace_id)
        .await
        .map_err(WorkspaceHarnessContainerError::Internal)
}

async fn stop_workspace_harness_container(
    handle: &WorkspaceHarnessContainerHandle,
    workspace_id: WorkspaceId,
) -> Result<(), WorkspaceHarnessContainerError> {
    ensure_workspace_exists(handle, workspace_id).await?;
    let stopped = handle
        .harness()
        .stop_container(workspace_id)
        .await
        .map_err(WorkspaceHarnessContainerError::Internal)?;
    if stopped {
        Ok(())
    } else {
        Err(WorkspaceHarnessContainerError::NotFound)
    }
}

async fn ensure_workspace_harness_container(
    handle: &WorkspaceHarnessContainerHandle,
    workspace_id: WorkspaceId,
) -> Result<(), WorkspaceHarnessContainerError> {
    let workspace = ensure_workspace_exists(handle, workspace_id).await?;
    let workspace_store = handle
        .store_for_workspace(workspace_id)
        .await
        .map_err(|error| {
            WorkspaceHarnessContainerError::ExecutionSettings(
                EffectiveExecutionSettingsError::Internal(error),
            )
        })?;
    let execution_settings = ctx_settings_service::effective_execution_settings_classified(
        handle.global_store(),
        &workspace_store,
    )
    .await
    .map_err(WorkspaceHarnessContainerError::ExecutionSettings)?;
    handle
        .harness()
        .ensure_workspace_container(&workspace, &execution_settings, handle.daemon_url())
        .await
        .map_err(WorkspaceHarnessContainerError::Ensure)
}

async fn ensure_workspace_exists(
    handle: &WorkspaceHarnessContainerHandle,
    workspace_id: WorkspaceId,
) -> Result<Workspace, WorkspaceHarnessContainerError> {
    handle
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(WorkspaceHarnessContainerError::Internal)?
        .ok_or(WorkspaceHarnessContainerError::NotFound)
}

fn workspace_harness_container_status_route_response(
    status: WorkspaceContainerStatus,
) -> WorkspaceHarnessContainerStatusRouteResponse {
    WorkspaceHarnessContainerStatusRouteResponse {
        name: status.name,
        running: status.running,
        known: status.known,
        mount_mode: status
            .mount_mode
            .map(workspace_harness_container_mount_mode_route_value),
        network_mode: status
            .network_mode
            .map(workspace_harness_container_network_mode_route_value),
        allowlist: status.allowlist,
        egress_guard: status.egress_guard,
    }
}

fn workspace_harness_container_mount_mode_route_value(
    mode: ContainerMountMode,
) -> WorkspaceHarnessContainerMountModeRouteValue {
    match mode {
        ContainerMountMode::DiskIsolated => {
            WorkspaceHarnessContainerMountModeRouteValue::DiskIsolated
        }
        ContainerMountMode::Legacy => WorkspaceHarnessContainerMountModeRouteValue::Legacy,
    }
}

fn workspace_harness_container_network_mode_route_value(
    mode: ContainerNetworkMode,
) -> WorkspaceHarnessContainerNetworkModeRouteValue {
    match mode {
        ContainerNetworkMode::LlmOnly => WorkspaceHarnessContainerNetworkModeRouteValue::LlmOnly,
        ContainerNetworkMode::Allowlist => {
            WorkspaceHarnessContainerNetworkModeRouteValue::Allowlist
        }
        ContainerNetworkMode::All => WorkspaceHarnessContainerNetworkModeRouteValue::All,
    }
}
