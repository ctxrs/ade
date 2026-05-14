use super::*;

pub(super) async fn collect_task_delete_cleanup_targets(
    handles: &TaskApiHandles,
    store: &Store,
    workspace: &Workspace,
    task: &Task,
    sessions: &[Session],
) -> Vec<TaskWorktreeCleanupTarget> {
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }
    let mut cleanup_targets = Vec::new();
    for worktree_id in &worktree_ids {
        let other_active = match store
            .count_active_tasks_for_worktree(*worktree_id, Some(task.id))
            .await
        {
            Ok(count) => count > 0,
            Err(err) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to check worktree usage: {err:#}"
                );
                true
            }
        };
        if other_active {
            continue;
        }
        let other_tasks = match store
            .count_tasks_for_worktree(*worktree_id, Some(task.id))
            .await
        {
            Ok(count) => count > 0,
            Err(err) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to check total worktree usage: {err:#}"
                );
                true
            }
        };
        let worktree = match store.get_worktree(*worktree_id).await {
            Ok(Some(worktree)) => worktree,
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load worktree for delete cleanup: {err:#}"
                );
                continue;
            }
        };
        let sandbox_binding = match store.get_sandbox_binding(*worktree_id).await {
            Ok(binding) => binding,
            Err(err) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load sandbox binding for delete cleanup: {err:#}"
                );
                None
            }
        };
        cleanup_targets.push(TaskWorktreeCleanupTarget {
            managed_root: handles
                .workspaces
                .managed_worktree_root(workspace, &worktree),
            sandbox_binding,
            worktree,
            destroy_worktree_on_cleanup: !other_tasks,
        });
    }
    cleanup_targets
}

pub(super) async fn delete_unused_worktree_records_after_cleanup(
    handles: &TaskApiHandles,
    store: &Store,
    task: &Task,
    cleanup_targets: &[TaskWorktreeCleanupTarget],
    cleanup_succeeded: bool,
) {
    if !cleanup_succeeded {
        return;
    }
    for target in cleanup_targets {
        if !target.destroy_worktree_on_cleanup {
            continue;
        }
        let deleted_worktree_row = match store.delete_worktree(target.worktree.id).await {
            Ok(deleted) => deleted,
            Err(err) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %target.worktree.id.0,
                    "failed to delete worktree row after task delete: {err:#}"
                );
                false
            }
        };
        if !deleted_worktree_row {
            tracing::warn!(
                task_id = %task.id.0,
                worktree_id = %target.worktree.id.0,
                "skipping worktree index deletion because worktree row was not deleted"
            );
            continue;
        }
        if let Err(err) = handles
            .sessions
            .delete_workspace_worktree_index(target.worktree.id)
            .await
        {
            tracing::warn!(
                task_id = %task.id.0,
                worktree_id = %target.worktree.id.0,
                "failed to delete worktree index after task delete: {err:#}"
            );
        }
    }
}
