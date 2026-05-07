use super::*;

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
            &workspace_effective,
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
        worktree_id
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
