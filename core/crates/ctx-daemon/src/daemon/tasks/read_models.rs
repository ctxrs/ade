use anyhow::Result;
use ctx_core::ids::{TaskId, WorkspaceId};
use ctx_core::models::{Session, Task, WorkspaceArchivedPage, WorkspaceIndexCursor};

use crate::daemon::handle::TasksHandle;
use crate::daemon::{workspaces, WorkspaceStoreAccessError};

impl TasksHandle {
    pub async fn list_workspace_tasks(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<Task>, WorkspaceStoreAccessError> {
        let store = self.state.existing_workspace_store(workspace_id).await?;
        ctx_task_service::read_models::list_workspace_tasks(&store, workspace_id)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn list_workspace_archived_page(
        &self,
        workspace_id: WorkspaceId,
        cursor: Option<WorkspaceIndexCursor>,
        limit: i64,
    ) -> Result<WorkspaceArchivedPage, WorkspaceStoreAccessError> {
        let store = self.state.existing_workspace_store(workspace_id).await?;
        let (_, archived_rev) =
            workspaces::load_workspace_active_snapshot_state(&self.state, workspace_id).await;
        ctx_task_service::read_models::list_workspace_archived_page(
            &store,
            workspace_id,
            cursor,
            limit,
            archived_rev,
        )
        .await
        .map_err(WorkspaceStoreAccessError::Unavailable)
    }

    pub async fn list_task_sessions(&self, task_id: TaskId) -> Result<Option<Vec<Session>>> {
        let Some(store) = self.task_store_or_none(task_id).await? else {
            return Ok(None);
        };
        ctx_task_service::read_models::list_task_sessions(&store, task_id)
            .await
            .map(Some)
    }
}
