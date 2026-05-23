use anyhow::Result;
use ctx_core::ids::TaskId;

use crate::daemon::handle::TasksHandle;
use crate::daemon::WorkspaceStoreAccessError;

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
}
