use std::sync::Arc;

use ctx_core::ids::WorkspaceId;

use crate::daemon::state::DaemonState;

use super::{cleanup_workspace_hooks, cleanup_worktree_hooks};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceDeleteError {
    NotFound,
    Internal,
}

pub async fn delete_workspace(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Result<(), WorkspaceDeleteError> {
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| WorkspaceDeleteError::Internal)?
        .ok_or(WorkspaceDeleteError::NotFound)?;
    let worktrees = match state.store_for_workspace(workspace_id).await {
        Ok(store) => store.list_worktrees(workspace_id).await.unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    state.core.stores.begin_workspace_delete(workspace_id).await;
    let delete_result = async {
        for worktree in &worktrees {
            if let Err(err) = cleanup_worktree_hooks(state.as_ref(), &workspace, worktree).await {
                tracing::warn!(
                    workspace_id = %workspace_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove vcs hooks: {err:#}"
                );
            }
        }
        state.cleanup_workspace(workspace_id).await;
        state
            .core
            .stores
            .evict_workspace_and_wait_closed(workspace_id)
            .await;
        state
            .global_store()
            .delete_workspace_indexes(workspace_id)
            .await
            .map_err(|_| WorkspaceDeleteError::Internal)?;
        state
            .global_store()
            .delete_workspace(workspace_id)
            .await
            .map_err(|_| WorkspaceDeleteError::Internal)?;
        Ok::<(), WorkspaceDeleteError>(())
    }
    .await;
    state
        .core
        .stores
        .finish_workspace_delete(workspace_id)
        .await;
    delete_result?;

    if let Err(err) = cleanup_workspace_hooks(state.as_ref(), workspace_id).await {
        tracing::warn!(
            workspace_id = %workspace_id.0,
            "failed to remove vcs hooks: {err:#}"
        );
    }

    let workspace_db_dir = state
        .core
        .data_root
        .join("db")
        .join("workspaces")
        .join(workspace_id.0.to_string());
    let _ = tokio::fs::remove_dir_all(workspace_db_dir).await;
    Ok(())
}
