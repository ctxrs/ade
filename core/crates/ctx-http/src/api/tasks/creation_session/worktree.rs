use super::*;
use create::create_session_execution_worktree;

#[path = "worktree/create.rs"]
mod create;

pub(super) struct SessionWorktreeResolution {
    pub(super) worktree_id: WorktreeId,
    pub(super) created_worktree_id: Option<WorktreeId>,
    pub(super) execution_environment: ExecutionEnvironment,
}

pub(super) async fn resolve_session_worktree_for_task(
    state: &Arc<AppState>,
    store: &Store,
    task: &Task,
    workspace: &Workspace,
    requested_worktree_id: Option<&str>,
    requested_execution_environment: Option<ExecutionEnvironment>,
) -> Result<SessionWorktreeResolution, StatusCode> {
    let workspace_effective =
        execution_effective::effective_execution_settings(state, workspace.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut existing_worktree = None;
    let worktree_id = if let Some(worktree_id) = requested_worktree_id {
        let worktree_id =
            WorktreeId(uuid::Uuid::parse_str(worktree_id).map_err(|_| StatusCode::BAD_REQUEST)?);
        existing_worktree = Some(
            resolve_existing_worktree_execution(state, store, workspace, worktree_id)
                .await
                .map_err(|_| StatusCode::NOT_FOUND)?,
        );
        worktree_id
    } else if let Some(primary) = task.primary_worktree_id {
        existing_worktree = Some(
            resolve_existing_worktree_execution(state, store, workspace, primary)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        );
        primary
    } else {
        create_session_execution_worktree(state, store, task, workspace, &workspace_effective)
            .await?
    };
    let created_worktree_id = existing_worktree.is_none().then_some(worktree_id);
    let execution_environment = if let Some(existing) = existing_worktree.as_ref() {
        let persisted = existing.execution_environment();
        if let Some(requested) = requested_execution_environment {
            if requested != persisted {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
        persisted
    } else {
        let effective_execution_environment =
            execution_environment_from_settings(&workspace_effective);
        match requested_execution_environment {
            Some(requested) => {
                if requested != effective_execution_environment {
                    if let Some(created_worktree_id) = created_worktree_id {
                        cleanup_orphaned_provisioned_worktree(
                            state,
                            store,
                            workspace,
                            task.id,
                            created_worktree_id,
                        )
                        .await;
                    }
                    return Err(StatusCode::BAD_REQUEST);
                }
                requested
            }
            None => effective_execution_environment,
        }
    };

    Ok(SessionWorktreeResolution {
        worktree_id,
        created_worktree_id,
        execution_environment,
    })
}
