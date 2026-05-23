use anyhow::Result;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::Workspace;
use ctx_store::Store;

use crate::daemon::handle::SessionsHandle;
use crate::daemon::workspaces::{
    complete_files_for_session, resolve_existing_worktree_execution, FileCompletionsError,
    ResolvedExistingWorktreeExecution,
};

impl SessionsHandle {
    pub async fn complete_files_for_session(
        &self,
        session_id: SessionId,
        query: Option<String>,
        limit: Option<u32>,
    ) -> Result<Vec<String>, FileCompletionsError> {
        complete_files_for_session(&self.state, session_id, query, limit).await
    }

    pub async fn resolve_existing_worktree_execution(
        &self,
        store: &Store,
        workspace: &Workspace,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<ResolvedExistingWorktreeExecution> {
        resolve_existing_worktree_execution(&self.state, store, workspace, worktree_id).await
    }

    pub async fn update_workspace_provider_preferred_model_id(
        &self,
        workspace_id: WorkspaceId,
        provider_id: &str,
        preferred_model_id: Option<String>,
    ) -> anyhow::Result<()> {
        let store = self.state.store_for_workspace(workspace_id).await?;
        ctx_workspace_config::update_preferred_new_session_model_id(
            &store,
            provider_id,
            preferred_model_id,
        )
        .await?;
        ctx_provider_runtime::provider_cache::invalidate_workspace_provider_options_cache(
            &self.state.providers,
            workspace_id,
            provider_id,
        )
        .await;
        Ok(())
    }

    pub async fn emit_workspace_task_upsert(&self, task_id: TaskId) -> anyhow::Result<()> {
        self.state.emit_workspace_task_upsert(task_id).await
    }
}
