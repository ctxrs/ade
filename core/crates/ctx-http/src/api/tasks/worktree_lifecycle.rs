use super::*;

mod git_ops;

pub(crate) use git_ops::{branch_exists, ensure_worktree_attached};
pub(super) use git_ops::{is_git_worktree, prune_worktrees, remove_worktree};

pub(crate) const GLOBAL_INDEX_WRITE_RETRY_LIMIT: usize = 3;
pub(crate) const GLOBAL_INDEX_WRITE_RETRY_BASE_MS: u64 = 40;

pub(crate) fn execution_environment_from_settings(
    settings: &ExecutionSettings,
) -> ExecutionEnvironment {
    match settings.mode {
        ExecutionMode::Host => ExecutionEnvironment::Host,
        ExecutionMode::Sandbox => ExecutionEnvironment::Sandbox,
    }
}

pub(crate) async fn materialize_sandbox_binding_for_worktree(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    canonical_root: &StdPath,
    effective: &ExecutionSettings,
    created_at: DateTime<Utc>,
) -> anyhow::Result<Option<SandboxBinding>> {
    let Some(materialization) = ctx_workspace_runtime::materialize_sandbox_worktree(
        &state.core.data_root,
        &state.core.daemon_url,
        state.execution.harness.as_ref(),
        workspace,
        worktree,
        canonical_root,
        effective,
    )
    .await?
    else {
        return Ok(None);
    };

    Ok(Some(SandboxBinding {
        worktree_id: worktree.id,
        workspace_id: workspace.id,
        sandbox_instance_id: materialization.sandbox_instance_id,
        substrate: materialization.substrate.substrate,
        guest_identity: materialization.substrate.guest_identity,
        profile: SandboxProfile::Standard,
        live_workspace_root: ctx_sandbox_contract::CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
        live_worktree_root: materialization
            .live_worktree_root
            .to_string_lossy()
            .to_string(),
        execution_settings_json: Some(serde_json::to_string(effective)?),
        container_name: Some(ctx_workspace_container::workspace_container_name(
            workspace.id,
        )),
        host_materialization_root: materialization
            .host_materialization_root
            .map(|path| path.to_string_lossy().to_string()),
        created_at,
    }))
}

pub(crate) async fn rematerialize_sandbox_binding_for_worktree(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    existing_binding: &SandboxBinding,
) -> anyhow::Result<SandboxBinding> {
    let canonical_root = managed_worktree_root(state, workspace, worktree)
        .ok_or_else(|| anyhow::anyhow!("worktree is not a managed ctx worktree"))?;
    let effective = sandbox_execution_settings_from_binding(existing_binding)?;
    materialize_sandbox_binding_for_worktree(
        state,
        workspace,
        worktree,
        &canonical_root,
        &effective,
        existing_binding.created_at,
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("sandbox binding rematerialization produced host mode"))
}

#[derive(Debug, Clone)]
pub(crate) struct TaskWorktreeCleanupTarget {
    pub worktree: Worktree,
    pub sandbox_binding: Option<SandboxBinding>,
    pub managed_root: Option<PathBuf>,
    pub destroy_worktree_on_cleanup: bool,
}

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
    let binding = materialize_sandbox_binding_for_worktree(
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

pub(crate) fn managed_worktree_root(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Option<PathBuf> {
    let root = PathBuf::from(&worktree.root_path);
    let expected = managed_worktree_path(&state.core.data_root, workspace.id, worktree.id);
    if normalize_path_for_comparison(&root) == normalize_path_for_comparison(&expected) {
        Some(expected)
    } else {
        None
    }
}

fn normalize_path_for_comparison(path: &StdPath) -> PathBuf {
    let mut suffix = Vec::new();
    let mut cursor = path;
    loop {
        match std::fs::canonicalize(cursor) {
            Ok(canonical) => {
                let mut normalized = canonical;
                for component in suffix.iter().rev() {
                    normalized.push(component);
                }
                return normalized;
            }
            Err(_) => {
                let Some(parent) = cursor.parent() else {
                    return path.to_path_buf();
                };
                let Some(name) = cursor.file_name() else {
                    return path.to_path_buf();
                };
                suffix.push(name.to_os_string());
                cursor = parent;
            }
        }
    }
}

pub(crate) async fn cleanup_task_worktrees(
    state: &AppState,
    workspace: &Workspace,
    task_id: TaskId,
    targets: &[TaskWorktreeCleanupTarget],
) -> Vec<anyhow::Error> {
    let mut errors = Vec::new();
    let mut needs_prune = false;
    let mut branches_to_delete = Vec::new();
    let workspace_root_exists = tokio::fs::metadata(&workspace.root_path).await.is_ok();
    for target in targets {
        let worktree = &target.worktree;
        if let Err(err) = vcs_hooks::cleanup_worktree_hooks(state, workspace, worktree).await {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to remove vcs hooks: {err:#}"
            );
        }
        if let Some(binding) = target.sandbox_binding.as_ref() {
            let sandbox_mode =
                match ctx_harness_runtime::selected_sandbox_command_mode(&state.core.data_root) {
                    Ok(mode) => mode,
                    Err(err) => {
                        tracing::warn!(
                            task_id = %task_id.0,
                            worktree_id = %worktree.id.0,
                            "failed to resolve sandbox command mode for cleanup: {err:#}"
                        );
                        errors.push(err);
                        continue;
                    }
                };
            if let Err(err) = ctx_sandbox_materialization::remove_live_worktree_root(
                &state.core.data_root,
                &sandbox_mode,
                workspace.id,
                StdPath::new(&binding.live_worktree_root),
            )
            .await
            {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    live_worktree_root = binding.live_worktree_root,
                    "failed to remove sandbox live worktree root: {err:#}"
                );
                errors.push(err);
            }
            if let Some(host_materialization_root) = binding.host_materialization_root.as_deref() {
                let host_materialization_root = PathBuf::from(host_materialization_root);
                if tokio::fs::metadata(&host_materialization_root)
                    .await
                    .is_ok()
                {
                    if let Err(err) = tokio::fs::remove_dir_all(&host_materialization_root)
                        .await
                        .with_context(|| {
                            format!(
                                "removing sandbox host materialization root at {}",
                                host_materialization_root.display()
                            )
                        })
                    {
                        tracing::warn!(
                            task_id = %task_id.0,
                            worktree_id = %worktree.id.0,
                            host_materialization_root = %host_materialization_root.display(),
                            "failed to remove sandbox host materialization root: {err:#}"
                        );
                        errors.push(err);
                    }
                }
            }
        }
        if !target.destroy_worktree_on_cleanup {
            continue;
        }
        let Some(root) = target.managed_root.as_ref() else {
            continue;
        };
        let branch = worktree
            .git_branch
            .as_deref()
            .filter(|name| name.starts_with("ctx/"));
        if !workspace_root_exists {
            if tokio::fs::metadata(root).await.is_ok() {
                if let Err(err) = tokio::fs::remove_dir_all(root).await.with_context(|| {
                    format!("removing orphaned worktree dir at {}", root.display())
                }) {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree.id.0,
                        workspace_root = %workspace.root_path,
                        "failed to remove orphaned worktree dir after workspace root disappeared: {err:#}"
                    );
                    errors.push(err);
                }
            }
            continue;
        }
        if tokio::fs::metadata(root).await.is_err() {
            if branch.is_some() {
                needs_prune = true;
            }
            if let Some(branch) = branch {
                branches_to_delete.push(branch.to_string());
            }
            continue;
        }
        let embedded_git_dir = tokio::fs::metadata(root.join(".git"))
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        let is_git = embedded_git_dir || is_git_worktree(root).await.unwrap_or(false);
        if embedded_git_dir {
            needs_prune = true;
            if let Err(err) = tokio::fs::remove_dir_all(root).await.with_context(|| {
                format!("removing standalone managed worktree at {}", root.display())
            }) {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove standalone managed worktree dir: {err:#}"
                );
                errors.push(err);
            }
        } else if is_git {
            needs_prune = true;
            if let Err(err) = remove_worktree(&workspace.root_path, root).await {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove worktree: {err:#}"
                );
                errors.push(err);
                continue;
            }
            if tokio::fs::metadata(root).await.is_ok() {
                if let Err(err) = tokio::fs::remove_dir_all(root)
                    .await
                    .with_context(|| format!("removing worktree dir at {}", root.display()))
                {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree.id.0,
                        "failed to remove worktree dir: {err:#}"
                    );
                    errors.push(err);
                }
            }
        } else if let Err(err) = tokio::fs::remove_dir_all(root)
            .await
            .with_context(|| format!("removing non-git worktree dir at {}", root.display()))
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to remove worktree dir: {err:#}"
            );
            errors.push(err);
        }
        if let Some(branch) = branch {
            branches_to_delete.push(branch.to_string());
        }
    }
    if needs_prune {
        if let Err(err) = prune_worktrees(&workspace.root_path).await {
            tracing::warn!(task_id = %task_id.0, "failed to prune worktrees: {err:#}");
            errors.push(err);
        }
    }
    branches_to_delete.sort();
    branches_to_delete.dedup();
    for branch in branches_to_delete {
        if let Err(err) = delete_branch(&workspace.root_path, &branch).await {
            tracing::warn!(
                task_id = %task_id.0,
                branch,
                "failed to delete worktree branch: {err:#}"
            );
        }
    }
    errors
}

fn is_transient_store_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("database is locked")
        || msg.contains("sqlite_busy")
        || msg.contains("database is busy")
}

pub(crate) async fn retry_global_index_write<Fut>(
    mut op: impl FnMut() -> Fut,
) -> Result<(), anyhow::Error>
where
    Fut: std::future::Future<Output = Result<(), anyhow::Error>>,
{
    let mut attempt = 0usize;
    loop {
        match op().await {
            Ok(()) => return Ok(()),
            Err(err) => {
                if !is_transient_store_error(&err) || attempt >= GLOBAL_INDEX_WRITE_RETRY_LIMIT {
                    return Err(err);
                }
                attempt += 1;
                let backoff_ms = GLOBAL_INDEX_WRITE_RETRY_BASE_MS.saturating_mul(attempt as u64);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
    }
}
