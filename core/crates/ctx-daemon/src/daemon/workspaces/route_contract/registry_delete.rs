use ctx_route_contracts::workspaces::WorkspaceRouteParams;

use super::super::{WorkspaceRouteError, WorkspacesHandle};
use super::common::workspace_delete_route_error;

impl WorkspacesHandle {
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
