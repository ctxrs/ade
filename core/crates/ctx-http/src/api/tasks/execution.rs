use super::*;

pub(in crate::api) struct ResolvedExistingWorktreeExecution {
    pub worktree: Worktree,
    pub effective: ExecutionSettings,
}

impl ResolvedExistingWorktreeExecution {
    pub fn execution_environment(&self) -> ExecutionEnvironment {
        execution_environment_from_settings(&self.effective)
    }
}

pub(in crate::api) async fn resolve_existing_worktree_execution(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    worktree_id: WorktreeId,
) -> anyhow::Result<ResolvedExistingWorktreeExecution> {
    let worktree = store
        .get_worktree(worktree_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worktree not found"))?;
    let base_effective =
        crate::execution_effective::effective_execution_settings(state, workspace.id)
            .await
            .context("loading workspace execution settings")?;
    let data_plane = crate::worktree_data_plane::resolve_worktree_data_plane(state, &worktree)
        .await
        .context("resolving worktree data plane")?;
    let effective = crate::worktree_data_plane::apply_data_plane_to_execution_settings(
        &base_effective,
        &data_plane,
    )
    .context("applying worktree data plane to execution settings")?;
    Ok(ResolvedExistingWorktreeExecution {
        worktree,
        effective,
    })
}

pub(super) fn sandbox_execution_settings_from_binding(
    binding: &SandboxBinding,
) -> anyhow::Result<ExecutionSettings> {
    if let Some(raw) = binding.execution_settings_json.as_deref() {
        return serde_json::from_str(raw).context("parsing sandbox binding execution settings");
    }

    let mut settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        ..ExecutionSettings::default()
    };
    settings.container.runtime = crate::worktree_data_plane::binding_runtime_kind(binding);
    settings.container.mount_mode = crate::settings::ContainerMountMode::DiskIsolated;
    Ok(settings)
}
