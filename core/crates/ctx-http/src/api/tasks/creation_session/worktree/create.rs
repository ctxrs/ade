use super::*;

pub(super) async fn create_session_execution_worktree(
    state: &Arc<AppState>,
    store: &Store,
    task: &Task,
    workspace: &Workspace,
    workspace_effective: &ExecutionSettings,
) -> Result<WorktreeId, StatusCode> {
    let workspace_root = StdPath::new(&workspace.root_path);
    let vcs = vcs::driver_for_path(workspace_root)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let base_commit_sha = vcs.rev_parse_head(workspace_root).await.map_err(|e| {
        let msg = e.to_string().to_lowercase();
        if msg.contains("ambiguous argument 'head'")
            || msg.contains("unknown revision or path not in the working tree")
        {
            return StatusCode::BAD_REQUEST;
        }
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let worktree_id = WorktreeId::new();
    let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
    let (wt_path, sandbox_binding) = provision_worktree_for_execution(
        state,
        workspace,
        worktree_id,
        &base_commit_sha,
        &branch_name,
        workspace_effective,
    )
    .await
    .map_err(|e| {
        tracing::warn!(
            task_id = %task.id.0,
            worktree_id = %worktree_id.0,
            "worktree provisioning failed: {e:#}"
        );
        shared::status_code_for_internal_error(&e)
    })?;

    let worktree = Worktree {
        id: worktree_id,
        workspace_id: task.workspace_id,
        root_path: wt_path.to_string_lossy().to_string(),
        base_commit_sha: base_commit_sha.clone(),
        git_branch: (vcs.kind() == VcsKind::Git).then(|| branch_name.clone()),
        vcs_kind: Some(vcs.kind()),
        base_revision: Some(base_commit_sha.clone()),
        vcs_ref: Some(branch_name.clone()),
        created_at: chrono::Utc::now(),
        bootstrap_status: None,
        bootstrap_started_at: None,
        bootstrap_finished_at: None,
        bootstrap_exit_code: None,
        bootstrap_timeout_sec: None,
        bootstrap_error: None,
        bootstrap_log_path: None,
        bootstrap_log_truncated: None,
        bootstrap_command: None,
        bootstrap_script_path: None,
    };
    persist_provisioned_worktree(state, store, workspace, worktree, sandbox_binding)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(worktree_id)
}
