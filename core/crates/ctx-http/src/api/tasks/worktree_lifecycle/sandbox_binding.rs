use super::*;
use ctx_sandbox_contract::sandbox_execution_settings_from_binding;

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
    Ok(ctx_workspace_runtime::materialize_sandbox_binding(
        &state.core.data_root,
        &state.core.daemon_url,
        state.execution.harness.as_ref(),
        workspace,
        worktree,
        canonical_root,
        effective,
        created_at,
    )
    .await?)
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
