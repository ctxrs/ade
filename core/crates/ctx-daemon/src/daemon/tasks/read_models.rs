use anyhow::Result;
use ctx_core::ids::{TaskId, WorkspaceId};
use ctx_core::models::{
    Session, Task, WorkspaceArchivedPage, WorkspaceIndexCursor, WorkspaceTaskSummary,
};

use crate::daemon::handle::TasksHandle;
use crate::daemon::{workspaces, WorkspaceStoreAccessError};

impl TasksHandle {
    pub async fn list_workspace_tasks(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<Task>, WorkspaceStoreAccessError> {
        let store = self.state.existing_workspace_store(workspace_id).await?;
        store
            .list_tasks(workspace_id)
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
        let (tasks, next_cursor): (Vec<WorkspaceTaskSummary>, Option<WorkspaceIndexCursor>) = store
            .list_workspace_archived_page(workspace_id, cursor, limit)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)?;
        let (_, total_archived) = store
            .workspace_task_counts(workspace_id)
            .await
            .map_err(WorkspaceStoreAccessError::Unavailable)?;
        let (_, archived_rev) =
            workspaces::load_workspace_active_snapshot_state(&self.state, workspace_id).await;

        Ok(WorkspaceArchivedPage {
            workspace_id,
            archived_rev,
            tasks,
            next_cursor,
            total_archived,
        })
    }

    pub async fn list_task_sessions(&self, task_id: TaskId) -> Result<Option<Vec<Session>>> {
        let Some(store) = self.task_store_or_none(task_id).await? else {
            return Ok(None);
        };
        store.list_sessions_for_task(task_id).await.map(Some)
    }
}
