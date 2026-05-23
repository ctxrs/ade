use anyhow::Result;
use ctx_core::ids::TaskId;
use ctx_core::models::{Task, Workspace};
use ctx_store::Store;

use crate::daemon::handle::TasksHandle;
use crate::daemon::WorkspaceStoreAccessError;

use super::TaskLifecycleError;

impl TasksHandle {
    pub(super) async fn task_store_or_none(
        &self,
        task_id: TaskId,
    ) -> Result<Option<ctx_store::Store>> {
        let Some(workspace_id) = self
            .state
            .global_store()
            .get_workspace_id_for_task(task_id)
            .await?
        else {
            return Ok(None);
        };
        match self.state.existing_workspace_store(workspace_id).await {
            Ok(store) => Ok(Some(store)),
            Err(WorkspaceStoreAccessError::NotFound) => Ok(None),
            Err(WorkspaceStoreAccessError::Unavailable(error)) => Err(error),
        }
    }

    pub(super) async fn load_task_context(
        &self,
        task_id: TaskId,
    ) -> Result<Option<(Store, Task, Workspace)>, TaskLifecycleError> {
        let Some(store) = self.task_store_or_none(task_id).await? else {
            return Ok(None);
        };
        let Some(task) = store
            .get_task(task_id)
            .await
            .map_err(TaskLifecycleError::Internal)?
        else {
            return Ok(None);
        };
        let workspace = self
            .state
            .global_store()
            .get_workspace(task.workspace_id)
            .await
            .map_err(TaskLifecycleError::Internal)?;
        let Some(workspace) = workspace else {
            return Ok(None);
        };
        Ok(Some((store, task, workspace)))
    }
}
