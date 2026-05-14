use super::*;

pub(super) struct UnarchiveWorktreePlan {
    pub(super) session_ids: Vec<SessionId>,
    pub(super) managed_worktrees: Vec<(Worktree, PathBuf)>,
    pub(super) worktrees: Vec<Worktree>,
}

pub(super) async fn load_unarchive_worktree_plan(
    handles: &TaskApiHandles,
    store: &Store,
    workspace: &Workspace,
    task: &Task,
) -> Result<UnarchiveWorktreePlan, StatusCode> {
    let task_id = task.id;
    let mut seen = HashSet::new();
    let mut managed_worktrees: Vec<(Worktree, PathBuf)> = Vec::new();
    let mut worktrees: Vec<Worktree> = Vec::new();
    let sessions = store
        .list_sessions_for_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let session_ids: Vec<SessionId> = sessions.iter().map(|session| session.id).collect();
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary) = task.primary_worktree_id {
        worktree_ids.insert(primary);
    }
    for worktree_id in worktree_ids {
        let worktree = store
            .get_worktree(worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if let Some(root) = handles
            .workspaces
            .managed_worktree_root(workspace, &worktree)
        {
            if seen.insert(worktree.id) {
                managed_worktrees.push((worktree.clone(), root));
            }
        }
        worktrees.push(worktree);
    }

    Ok(UnarchiveWorktreePlan {
        session_ids,
        managed_worktrees,
        worktrees,
    })
}
