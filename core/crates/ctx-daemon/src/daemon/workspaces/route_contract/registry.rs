use ctx_core::ids::WorkspaceId;

use super::super::{WorkspaceRouteError, WorkspacesHandle};
use super::common::{workspace_delete_route_error, WorkspaceRouteParams};
use super::responses::WorkspaceRouteResponse;

impl WorkspacesHandle {
    pub async fn list_workspaces_for_route(
        &self,
    ) -> Result<Vec<WorkspaceRouteResponse>, WorkspaceRouteError> {
        let workspaces = self
            .list_workspaces()
            .await
            .map_err(WorkspaceRouteError::internal)?;
        Ok(workspaces.into_iter().map(Into::into).collect())
    }

    pub async fn get_workspace_for_route_params(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<WorkspaceRouteResponse, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.get_workspace_for_route(workspace_id)
            .await?
            .ok_or_else(|| WorkspaceRouteError::not_found("workspace not found"))
    }

    pub async fn get_workspace_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceRouteResponse>, WorkspaceRouteError> {
        let workspace = self
            .get_workspace(workspace_id)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        if workspace.is_some() {
            self.record_workspace_opened().await;
        }
        Ok(workspace.map(Into::into))
    }

    pub async fn delete_workspace_for_route(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<(), WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.delete_workspace(workspace_id)
            .await
            .map_err(workspace_delete_route_error)
    }
}
