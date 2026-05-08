use super::*;

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
    let canonical_root = super::managed_worktree_root(state, workspace, worktree)
        .ok_or_else(|| anyhow::anyhow!("worktree is not a managed ctx worktree"))?;
    materialize_sandbox_binding_for_worktree(
        state,
        workspace,
        worktree,
        &canonical_root,
        &sandbox_execution_settings_from_binding(existing_binding)?,
        existing_binding.created_at,
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("sandbox binding rematerialization produced host mode"))
}
