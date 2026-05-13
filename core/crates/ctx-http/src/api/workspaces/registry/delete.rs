use super::*;
use ctx_workspace_services::vcs_hooks::cleanup_workspace_hooks;

pub(in crate::api) async fn delete_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let workspace = state
        .global_store()
        .get_workspace(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let worktrees = match state.store_for_workspace(id).await {
        Ok(store) => store.list_worktrees(id).await.unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    state.core.stores.begin_workspace_delete(id).await;
    let delete_result = async {
        for worktree in &worktrees {
            if let Err(err) =
                vcs_hooks::cleanup_worktree_hooks(state.as_ref(), &workspace, worktree).await
            {
                tracing::warn!(
                    workspace_id = %id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove vcs hooks: {err:#}"
                );
            }
        }
        state.cleanup_workspace(id).await;
        state.core.stores.evict_workspace_and_wait_closed(id).await;
        state
            .global_store()
            .delete_workspace_indexes(id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        state
            .global_store()
            .delete_workspace(id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok::<(), StatusCode>(())
    }
    .await;
    state.core.stores.finish_workspace_delete(id).await;
    delete_result?;
    if let Err(err) = cleanup_workspace_hooks(&state.core.data_root, id).await {
        tracing::warn!(
            workspace_id = %id.0,
            "failed to remove vcs hooks: {err:#}"
        );
    }
    let workspace_db_dir = state
        .core
        .data_root
        .join("db")
        .join("workspaces")
        .join(id.0.to_string());
    let _ = tokio::fs::remove_dir_all(workspace_db_dir).await;
    Ok(StatusCode::NO_CONTENT)
}
