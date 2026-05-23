use ctx_core::ids::WorkspaceId;
use ctx_observability::telemetry::TelemetryEvent;
use ctx_repo_onboarding_service::prepare_workspace_registration;
use ctx_route_contracts::workspaces::{
    CreateWorkspaceRequest, WorkspaceRouteParams, WorkspaceRouteResponse,
};
use ctx_workspace_config as workspace_config;

use super::super::{workspace_store_route_error, WorkspaceRouteError};
use crate::daemon::WorkspaceRegistryHandle;

impl WorkspaceRegistryHandle {
    pub async fn list_workspaces_for_route(
        &self,
    ) -> Result<Vec<WorkspaceRouteResponse>, WorkspaceRouteError> {
        let workspaces = self
            .global_store()
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
            .global_store()
            .get_workspace(workspace_id)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        if workspace.is_some() {
            self.telemetry()
                .emit(TelemetryEvent::workspace_opened())
                .await;
        }
        Ok(workspace.map(Into::into))
    }

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
            .global_store()
            .create_workspace(name, root_path, candidate.vcs_kind)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        let store = self
            .existing_workspace_store(workspace.id)
            .await
            .map_err(workspace_store_route_error)?;
        workspace_config::update_primary_branch(&store, &candidate.primary_branch)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        self.telemetry()
            .emit(TelemetryEvent::workspace_registered())
            .await;
        Ok(workspace.into())
    }
}
