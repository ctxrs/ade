use anyhow::{anyhow, Result};
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::Workspace;
use ctx_store::Store;

use crate::daemon::handle::SessionsHandle;
use crate::daemon::SessionStoreAccessError;

pub struct WorkspaceStoreContext {
    pub workspace: Workspace,
    pub store: Store,
}

impl SessionsHandle {
    pub async fn get_workspace(&self, workspace_id: WorkspaceId) -> Result<Option<Workspace>> {
        self.state.global_store().get_workspace(workspace_id).await
    }

    pub async fn get_workspace_id_for_task(&self, task_id: TaskId) -> Result<Option<WorkspaceId>> {
        self.state
            .global_store()
            .get_workspace_id_for_task(task_id)
            .await
    }

    pub async fn get_workspace_id_for_session(
        &self,
        session_id: SessionId,
    ) -> Result<Option<WorkspaceId>> {
        self.state
            .global_store()
            .get_workspace_id_for_session(session_id)
            .await
    }

    pub async fn upsert_workspace_task_index(
        &self,
        task_id: TaskId,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        self.state
            .global_store()
            .upsert_workspace_task_index(task_id, workspace_id)
            .await
    }

    pub async fn upsert_workspace_session_index(
        &self,
        session_id: SessionId,
        workspace_id: WorkspaceId,
    ) -> Result<()> {
        self.state
            .global_store()
            .upsert_workspace_session_index(session_id, workspace_id)
            .await
    }

    pub async fn delete_workspace_worktree_index(&self, worktree_id: WorktreeId) -> Result<()> {
        self.state
            .global_store()
            .delete_workspace_worktree_index(worktree_id)
            .await
    }

    pub async fn load_workspace_context(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceStoreContext>> {
        let Some(workspace) = self.get_workspace(workspace_id).await? else {
            return Ok(None);
        };
        let store = self.store_for_workspace(workspace_id).await?;
        Ok(Some(WorkspaceStoreContext { workspace, store }))
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store> {
        self.state.store_for_workspace(workspace_id).await
    }

    pub(in crate::daemon) async fn store_for_session(
        &self,
        session_id: SessionId,
    ) -> Result<Store> {
        self.state.store_for_session(session_id).await
    }

    pub(in crate::daemon) async fn existing_session_store(
        &self,
        session_id: SessionId,
    ) -> Result<Store, SessionStoreAccessError> {
        self.state.existing_session_store(session_id).await
    }

    pub(super) async fn session_store_or_none(
        &self,
        session_id: SessionId,
    ) -> Result<Option<Store>> {
        match self.existing_session_store(session_id).await {
            Ok(store) => Ok(Some(store)),
            Err(SessionStoreAccessError::NotFound) => Ok(None),
            Err(error) => Err(session_store_access_anyhow(error)),
        }
    }
}

fn session_store_access_anyhow(error: SessionStoreAccessError) -> anyhow::Error {
    match error {
        SessionStoreAccessError::NotFound => anyhow!("session not found"),
        SessionStoreAccessError::LookupUnavailable(error) => error,
        SessionStoreAccessError::StoreUnavailable => anyhow!("workspace store unavailable"),
    }
}
