use ctx_core::ids::WorktreeId;
use ctx_route_contracts::downloads::TextRouteDownload;
use ctx_route_contracts::workspaces::{
    WorkspaceFileCompletionsRouteQuery, WorkspaceRouteParams, WorktreeRouteParams,
    WorktreeRouteResponse,
};

use super::super::{WorkspaceRouteError, WorkspacesHandle};
use super::common::{file_completions_route_error, route_file_download_error};

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

    pub async fn workspace_file_completions_for_route(
        &self,
        params: WorkspaceRouteParams,
        query: WorkspaceFileCompletionsRouteQuery,
    ) -> Result<Vec<String>, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        let (query, limit) = query.into_parts();
        self.complete_files_for_workspace(workspace_id, query, limit)
            .await
            .map_err(file_completions_route_error)
    }
}
