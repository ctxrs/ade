use async_trait::async_trait;
use ctx_core::ids::WorktreeId;
use ctx_core::models::Workspace;
use ctx_route_contracts::downloads::TextRouteDownload;
use ctx_route_contracts::workspaces::{WorktreeRouteParams, WorktreeRouteResponse};
use ctx_store::Store;
use ctx_worktree_data_plane::WorktreeDataPlaneHost;

use crate::daemon::{RouteFileDownloadError, WorkspaceStoreAccessError, WorkspaceWorktreeHandle};

use super::super::WorkspaceRouteError;
use super::common::route_file_download_error;

impl WorkspaceWorktreeHandle {
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
        get_worktree_with_live_root(self, worktree_id)
            .await
            .map(|worktree| worktree.map(Into::into))
            .map_err(WorkspaceRouteError::internal)
    }

    pub async fn download_worktree_bootstrap_logs_for_route_params(
        &self,
        params: WorktreeRouteParams,
    ) -> Result<TextRouteDownload, WorkspaceRouteError> {
        let worktree_id = params.parse_worktree_id()?;
        download_worktree_bootstrap_logs_for_route(self, worktree_id)
            .await
            .map_err(route_file_download_error)
    }
}

#[async_trait]
impl WorktreeDataPlaneHost for WorkspaceWorktreeHandle {
    async fn get_workspace(
        handle: &Self,
        workspace_id: ctx_core::ids::WorkspaceId,
    ) -> anyhow::Result<Option<Workspace>> {
        handle.global_store().get_workspace(workspace_id).await
    }

    async fn workspace_store(
        handle: &Self,
        workspace_id: ctx_core::ids::WorkspaceId,
    ) -> anyhow::Result<Store> {
        handle.store_for_workspace(workspace_id).await
    }
}

async fn get_worktree_with_live_root(
    handle: &WorkspaceWorktreeHandle,
    worktree_id: WorktreeId,
) -> anyhow::Result<Option<ctx_core::models::Worktree>> {
    let Some(store) = worktree_store_or_none(handle, worktree_id).await? else {
        return Ok(None);
    };
    let Some(mut worktree) = store.get_worktree(worktree_id).await? else {
        return Ok(None);
    };
    worktree.root_path =
        ctx_worktree_data_plane::resolve_worktree_data_plane_with_host(handle, &worktree)
            .await?
            .live_worktree_root
            .to_string_lossy()
            .to_string();
    Ok(Some(worktree))
}

async fn get_worktree_bootstrap_log_path(
    handle: &WorkspaceWorktreeHandle,
    worktree_id: WorktreeId,
) -> anyhow::Result<Option<String>> {
    let Some(store) = worktree_store_or_none(handle, worktree_id).await? else {
        return Ok(None);
    };
    let Some(worktree) = store.get_worktree(worktree_id).await? else {
        return Ok(None);
    };
    Ok(worktree.bootstrap_log_path)
}

async fn download_worktree_bootstrap_logs_for_route(
    handle: &WorkspaceWorktreeHandle,
    worktree_id: WorktreeId,
) -> Result<TextRouteDownload, RouteFileDownloadError> {
    let path = get_worktree_bootstrap_log_path(handle, worktree_id)
        .await
        .map_err(|_| RouteFileDownloadError::Internal)?
        .ok_or(RouteFileDownloadError::NotFound)?;
    if path.trim().is_empty() {
        return Err(RouteFileDownloadError::NotFound);
    }
    let log_root = ctx_observability::logs::logs_dir(handle.data_root()).join("worktree-bootstrap");
    crate::daemon::route_files::read_text_route_file(
        std::path::Path::new(&path),
        &log_root,
        format!("worktree-bootstrap-{}.log", worktree_id.0),
    )
    .await
}

async fn worktree_store_or_none(
    handle: &WorkspaceWorktreeHandle,
    worktree_id: WorktreeId,
) -> anyhow::Result<Option<Store>> {
    let Some(workspace_id) = handle
        .global_store()
        .get_workspace_id_for_worktree(worktree_id)
        .await?
    else {
        return Ok(None);
    };
    match handle.existing_workspace_store(workspace_id).await {
        Ok(store) => Ok(Some(store)),
        Err(WorkspaceStoreAccessError::NotFound) => Ok(None),
        Err(WorkspaceStoreAccessError::Unavailable(error)) => Err(error),
    }
}
