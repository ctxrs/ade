use ctx_core::ids::WorktreeId;
use ctx_route_contracts::downloads::TextRouteDownload;
use ctx_route_contracts::workspaces::{WorktreeRouteParams, WorktreeRouteResponse};

use super::super::{WorkspaceRouteError, WorkspacesHandle};
use super::common::route_file_download_error;

impl WorkspacesHandle {
    pub async fn get_worktree_for_route_params(
        &self,
        params: WorktreeRouteParams,
    ) -> Result<WorktreeRouteResponse, WorkspaceRouteError> {
        let worktree_id = params.parse_worktree_id()?;
        self.get_worktree_for_route(worktree_id)
            .await?
            .ok_or_else(|| WorkspaceRouteError::not_found("worktree not found"))
    }

    pub async fn get_worktree_for_route(
        &self,
        worktree_id: WorktreeId,
    ) -> Result<Option<WorktreeRouteResponse>, WorkspaceRouteError> {
        self.get_worktree_with_live_root(worktree_id)
            .await
            .map(|worktree| worktree.map(Into::into))
            .map_err(WorkspaceRouteError::internal)
    }

    pub async fn download_worktree_bootstrap_logs_for_route_params(
        &self,
        params: WorktreeRouteParams,
    ) -> Result<TextRouteDownload, WorkspaceRouteError> {
        let worktree_id = params.parse_worktree_id()?;
        self.download_worktree_bootstrap_logs_for_route(worktree_id)
            .await
            .map_err(route_file_download_error)
    }
}
