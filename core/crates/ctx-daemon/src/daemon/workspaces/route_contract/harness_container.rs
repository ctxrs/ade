use ctx_core::ids::WorkspaceId;

use super::super::{WorkspaceHarnessContainerError, WorkspaceRouteError, WorkspacesHandle};
use super::common::{
    workspace_harness_container_ensure_error, workspace_harness_container_status_error,
    WorkspaceRouteParams,
};
use super::responses::WorkspaceHarnessContainerStatusRouteResponse;

impl WorkspacesHandle {
    pub async fn workspace_harness_container_status_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceHarnessContainerStatusRouteResponse>, WorkspaceHarnessContainerError>
    {
        self.workspace_harness_container_status(workspace_id)
            .await
            .map(|status| status.map(Into::into))
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
        self.stop_workspace_harness_container(workspace_id)
            .await
            .map_err(workspace_harness_container_status_error)
    }

    pub async fn ensure_workspace_harness_container_for_route(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<(), WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.ensure_workspace_harness_container(workspace_id)
            .await
            .map_err(workspace_harness_container_ensure_error)
    }
}
