use super::*;

pub(crate) async fn persist_provisioned_worktree(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    worktree: Worktree,
    sandbox_binding: Option<SandboxBinding>,
) -> anyhow::Result<Worktree> {
    store.insert_worktree(worktree.clone()).await?;
    if let Some(binding) = sandbox_binding {
        store.upsert_sandbox_binding(binding).await?;
    }
    retry_global_index_write(|| async {
        state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, workspace.id)
            .await
    })
    .await?;

    if let Err(err) = worktree_bootstrap::spawn_worktree_bootstrap(
        Arc::clone(state),
        workspace.clone(),
        worktree.clone(),
    )
    .await
    {
        tracing::warn!(worktree_id = %worktree.id.0, "worktree bootstrap failed: {err:?}");
    }
    if let Err(err) = crate::daemon::workspaces::attachments::sync_workspace_attachments(
        Arc::clone(state),
        workspace,
        false,
    )
    .await
    {
        tracing::warn!(worktree_id = %worktree.id.0, "attachment sync failed: {err:?}");
    }
    if let Err(err) =
        crate::daemon::workspaces::attachments::ensure_worktree_attachment_mounts_if_materialized(
            state, workspace, &worktree,
        )
        .await
    {
        tracing::warn!(worktree_id = %worktree.id.0, "attachment mounts failed: {err:?}");
    }

    Ok(worktree)
}

pub(crate) async fn provision_worktree_for_execution(
    state: &Arc<AppState>,
    workspace: &Workspace,
    worktree_id: WorktreeId,
    base_commit_sha: &str,
    branch_name: &str,
    effective: &ExecutionSettings,
) -> anyhow::Result<(PathBuf, Option<SandboxBinding>)> {
    let canonical_root = managed_worktree_path(&state.core.data_root, workspace.id, worktree_id);
    if let Some(parent) = canonical_root.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    create_worktree(
        &workspace.root_path,
        &canonical_root,
        base_commit_sha,
        branch_name,
    )
    .await?;

    let worktree = Worktree {
        id: worktree_id,
        workspace_id: workspace.id,
        root_path: canonical_root.to_string_lossy().to_string(),
        base_commit_sha: base_commit_sha.to_string(),
        git_branch: Some(branch_name.to_string()),
        vcs_kind: Some(VcsKind::Git),
        base_revision: Some(base_commit_sha.to_string()),
        vcs_ref: Some(branch_name.to_string()),
        created_at: Utc::now(),
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
    let binding = super::sandbox_binding::materialize_sandbox_binding_for_worktree(
        state,
        workspace,
        &worktree,
        &canonical_root,
        effective,
        Utc::now(),
    )
    .await?;

    Ok((canonical_root, binding))
}
