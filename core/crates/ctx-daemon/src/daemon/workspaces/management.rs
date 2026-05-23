use std::path::{Path, PathBuf};

use ctx_core::ids::{MergeQueueEntryId, WorkspaceId};
use ctx_core::models::{MergeQueueRun, Workspace, Worktree};
use ctx_repo_onboarding_service::validate_workspace_primary_branch;
use ctx_route_contracts::downloads::TextRouteDownload;
use ctx_store::Store;
use ctx_workspace_config as workspace_config;

use super::route_config::{
    merge_queue_config_route_response, merge_queue_config_update, request_or_policy_route_error,
    workspace_store_route_error, UpdateWorkspaceMergeQueueConfigRequest,
    UpdateWorkspacePrimaryBranchRequest, WorkspaceConfigUpdateResult,
    WorkspaceMergeQueueConfigRouteResponse, WorkspacePrimaryBranchSnapshot, WorkspaceRouteError,
};
use crate::daemon::route_files::{read_text_route_file, RouteFileDownloadError};
use crate::daemon::{WorkspaceStoreAccessError, WorkspacesHandle};

impl WorkspacesHandle {
    pub async fn update_workspace_primary_branch(
        &self,
        workspace_id: WorkspaceId,
        primary_branch: &str,
    ) -> anyhow::Result<()> {
        let store = self.store_for_workspace(workspace_id).await?;
        ctx_workspace_config::update_primary_branch(&store, primary_branch).await
    }

    pub async fn load_workspace_primary_branch_config(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<String>, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        workspace_config::load_primary_branch(&store)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn update_workspace_primary_branch_config(
        &self,
        workspace: &Workspace,
        primary_branch: &str,
    ) -> anyhow::Result<()> {
        let store = self.store_for_workspace(workspace.id).await?;
        workspace_config::update_primary_branch(&store, primary_branch).await?;
        let worktrees = store.list_worktrees(workspace.id).await?;
        for worktree in worktrees {
            if let Err(error) = self.refresh_worktree_vcs_snapshot(&worktree, true).await {
                tracing::warn!(
                    workspace_id = %workspace.id.0,
                    worktree_id = %worktree.id.0,
                    "failed to refresh worktree vcs after primary branch update: {error:#}"
                );
            }
        }
        Ok(())
    }

    pub async fn workspace_primary_branch_for_request(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspacePrimaryBranchSnapshot, WorkspaceRouteError> {
        let primary_branch = self
            .load_workspace_primary_branch_config(workspace_id)
            .await
            .map_err(workspace_store_route_error)?
            .ok_or_else(|| {
                WorkspaceRouteError::not_found("workspace primary branch is not configured")
            })?;
        Ok(WorkspacePrimaryBranchSnapshot { primary_branch })
    }

    pub async fn update_workspace_primary_branch_for_request(
        &self,
        workspace_id: WorkspaceId,
        req: UpdateWorkspacePrimaryBranchRequest,
    ) -> Result<WorkspacePrimaryBranchSnapshot, WorkspaceRouteError> {
        let workspace = self
            .state
            .global_store()
            .get_workspace(workspace_id)
            .await
            .map_err(WorkspaceRouteError::internal)?
            .ok_or_else(|| WorkspaceRouteError::not_found("workspace not found"))?;
        let primary_branch =
            validate_workspace_primary_branch(Path::new(&workspace.root_path), &req.primary_branch)
                .await
                .map_err(|error| WorkspaceRouteError::bad_request(error.message()))?;
        let store = self
            .existing_workspace_store(workspace.id)
            .await
            .map_err(workspace_store_route_error)?;
        let primary_branch =
            workspace_config::update_and_load_primary_branch(&store, &primary_branch)
                .await
                .map_err(WorkspaceRouteError::internal)?;
        let worktrees = store
            .list_worktrees(workspace.id)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        for worktree in worktrees {
            if let Err(error) = self.refresh_worktree_vcs_snapshot(&worktree, true).await {
                tracing::warn!(
                    workspace_id = %workspace.id.0,
                    worktree_id = %worktree.id.0,
                    "failed to refresh worktree vcs after primary branch update: {error:#}"
                );
            }
        }
        Ok(WorkspacePrimaryBranchSnapshot { primary_branch })
    }

    pub async fn latest_merge_queue_run_for_route(
        &self,
        workspace_id: WorkspaceId,
        entry_id: MergeQueueEntryId,
    ) -> Result<Option<(Workspace, MergeQueueRun)>, WorkspaceStoreAccessError> {
        let store = self.existing_workspace_store(workspace_id).await?;
        let Some(workspace) = store
            .get_workspace(workspace_id)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)?
        else {
            return Ok(None);
        };
        let run = store
            .get_latest_merge_queue_run(entry_id)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)?;
        Ok(run.map(|run| (workspace, run)))
    }

    pub async fn download_merge_queue_entry_logs_for_route(
        &self,
        workspace_id: WorkspaceId,
        entry_id: MergeQueueEntryId,
    ) -> Result<TextRouteDownload, RouteFileDownloadError> {
        self.get_workspace_merge_queue_entry(workspace_id, entry_id)
            .await
            .map_err(|_| RouteFileDownloadError::NotFound)?;
        let (workspace, run) = self
            .latest_merge_queue_run_for_route(workspace_id, entry_id)
            .await
            .map_err(|_| RouteFileDownloadError::Internal)?
            .ok_or(RouteFileDownloadError::NotFound)?;
        let Some(path) = run.log_path.as_deref() else {
            return Err(RouteFileDownloadError::NotFound);
        };
        if path.trim().is_empty() {
            return Err(RouteFileDownloadError::NotFound);
        }
        let log_root = PathBuf::from(&workspace.root_path)
            .join(".ctx")
            .join("merge-queue")
            .join("logs");
        read_text_route_file(
            Path::new(path),
            &log_root,
            format!("merge-queue-{}.log", entry_id.0),
        )
        .await
    }

    pub async fn workspace_merge_queue_config_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceMergeQueueConfigRouteResponse, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(workspace_store_route_error)?;
        let cfg = workspace_config::load_merge_queue_config(&store)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        Ok(merge_queue_config_route_response(cfg))
    }

    pub async fn update_workspace_merge_queue_config_for_route(
        &self,
        workspace_id: WorkspaceId,
        req: UpdateWorkspaceMergeQueueConfigRequest,
    ) -> Result<WorkspaceConfigUpdateResult, WorkspaceRouteError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(workspace_store_route_error)?;
        let transition = workspace_config::update_merge_queue_config_with_transition(
            &store,
            merge_queue_config_update(req),
        )
        .await
        .map_err(request_or_policy_route_error)?;
        if !transition.was_enabled && transition.now_enabled {
            self.schedule_workspace_merge_queue_if_enabled_and_queued(workspace_id)
                .await
                .map_err(request_or_policy_route_error)?;
        } else if transition.was_enabled && !transition.now_enabled {
            self.cancel_queued_entries_for_disabled_workspace(&store, workspace_id)
                .await
                .map_err(request_or_policy_route_error)?;
        }
        Ok(WorkspaceConfigUpdateResult { ok: true })
    }

    #[cfg(target_os = "macos")]
    pub fn shared_vm_container_runtime_available(&self) -> bool {
        ctx_harness_runtime::local_runtime_available(
            &self.state.core.data_root,
            &ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
        )
    }

    pub async fn refresh_worktree_vcs_snapshot(
        &self,
        worktree: &Worktree,
        force_emit: bool,
    ) -> anyhow::Result<()> {
        crate::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
            &self.state,
            worktree,
            force_emit,
        )
        .await
    }

    pub async fn schedule_workspace_merge_queue_if_enabled_and_queued(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<bool> {
        crate::daemon::merge_queue::schedule_workspace_if_enabled_and_queued(
            &self.state,
            workspace_id,
        )
        .await
    }

    pub async fn cancel_queued_entries_for_disabled_workspace(
        &self,
        store: &Store,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<()> {
        crate::daemon::merge_queue::cancel_queued_entries_for_disabled_workspace(
            &self.state,
            store,
            workspace_id,
        )
        .await
    }
}
