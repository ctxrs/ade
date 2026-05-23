use std::path::{Path, PathBuf};

use ctx_core::ids::{MergeQueueEntryId, WorkspaceId};
use ctx_core::models::{MergeQueueRun, Workspace};
use ctx_route_contracts::downloads::TextRouteDownload;

use crate::daemon::route_files::{read_text_route_file, RouteFileDownloadError};
use crate::daemon::{WorkspaceStoreAccessError, WorkspacesHandle};

impl WorkspacesHandle {
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

    #[cfg(target_os = "macos")]
    pub fn shared_vm_container_runtime_available(&self) -> bool {
        ctx_harness_runtime::local_runtime_available(
            &self.state.core.data_root,
            &ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
        )
    }
}
