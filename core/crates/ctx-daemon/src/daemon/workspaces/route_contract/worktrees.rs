use ctx_core::ids::WorktreeId;
use serde::Deserialize;

use crate::daemon::TextRouteDownload;

use super::super::{WorkspaceRouteError, WorkspacesHandle};
use super::common::{
    file_completions_route_error, route_file_download_error, WorkspaceRouteParams,
    WorktreeRouteParams,
};
use super::responses::WorktreeRouteResponse;

#[derive(Debug, Clone, Deserialize, Default, Eq, PartialEq)]
pub struct WorkspaceFileCompletionsRouteQuery {
    #[serde(default)]
    pub(super) query: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<u32>,
}

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
        self.complete_files_for_workspace(workspace_id, query.query, query.limit)
            .await
            .map_err(file_completions_route_error)
    }
}
