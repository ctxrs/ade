use std::collections::HashMap;
use std::path::PathBuf;

use ctx_core::models::{Workspace, Worktree};
use ctx_observability::logs;
use ctx_provider_runtime::provider_launch::probe::PreparedWorkspaceProbeRuntime;
use ctx_settings_model::ExecutionMode;
use ctx_worktree_data_plane::{
    apply_data_plane_to_execution_settings, resolve_worktree_data_plane_with_host,
    workspace_data_plane,
};

use crate::daemon::execution_effective;
use crate::daemon::DaemonState;

use self::helpers::{probe_cwd_for_workspace_runtime, runtime_data_root, synthetic_probe_worktree};

mod helpers;

pub(super) async fn prepare_workspace_probe_runtime(
    state: &DaemonState,
    workspace: &Workspace,
) -> Result<PreparedWorkspaceProbeRuntime, String> {
    let effective = execution_effective::effective_execution_settings(state, workspace.id)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("effective execution settings failed: {err}"))
        })?;
    if matches!(effective.mode, ExecutionMode::Host) {
        return Ok(PreparedWorkspaceProbeRuntime {
            cwd: PathBuf::from(&workspace.root_path),
            runtime_data_root: None,
            env_overrides: HashMap::new(),
        });
    }

    let worktree = synthetic_probe_worktree(workspace);
    let worktree_data_plane = workspace_data_plane(workspace, effective.mode.clone());
    let effective = apply_data_plane_to_execution_settings(&effective, &worktree_data_plane)
        .map_err(|err| {
            logs::redact_sensitive(&format!(
                "applying workspace probe data plane failed: {err:#}"
            ))
        })?;
    let cwd = probe_cwd_for_workspace_runtime(
        &worktree_data_plane,
        &worktree,
        effective.mode.clone(),
        effective.container.mount_mode.clone(),
    );
    let runtime_plan = state
        .execution
        .harness
        .prepare(workspace, &worktree, &effective, &state.core.daemon_url)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("probe runtime preparation failed: {err:#}"))
        })?;
    let sandbox_mode = ctx_harness_runtime::selected_sandbox_command_mode(&state.core.data_root)
        .map_err(|err| {
            logs::redact_sensitive(&format!("sandbox command selection failed: {err:#}"))
        })?;
    ctx_sandbox_materialization::ensure_workspace_root_from_host_copy(
        &state.core.data_root,
        &sandbox_mode,
        workspace,
    )
    .await
    .map_err(|err| {
        logs::redact_sensitive(&format!(
            "sandbox workspace root materialization failed: {err:#}"
        ))
    })?;
    let runtime_data_root = runtime_data_root(&runtime_plan.env_overrides);
    Ok(PreparedWorkspaceProbeRuntime {
        cwd,
        runtime_data_root,
        env_overrides: runtime_plan.env_overrides,
    })
}

pub(super) async fn prepare_worktree_probe_runtime(
    state: &DaemonState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<PreparedWorkspaceProbeRuntime, String> {
    let effective = execution_effective::effective_execution_settings(state, workspace.id)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("effective execution settings failed: {err}"))
        })?;
    if matches!(effective.mode, ExecutionMode::Host) {
        return Ok(PreparedWorkspaceProbeRuntime {
            cwd: PathBuf::from(&worktree.root_path),
            runtime_data_root: None,
            env_overrides: HashMap::new(),
        });
    }

    let worktree_data_plane = resolve_worktree_data_plane_with_host(state, worktree)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!(
                "resolving session auth worktree data plane failed: {err:#}"
            ))
        })?;
    let effective = apply_data_plane_to_execution_settings(&effective, &worktree_data_plane)
        .map_err(|err| {
            logs::redact_sensitive(&format!(
                "applying session auth worktree data plane failed: {err:#}"
            ))
        })?;
    let cwd = probe_cwd_for_workspace_runtime(
        &worktree_data_plane,
        worktree,
        effective.mode.clone(),
        effective.container.mount_mode.clone(),
    );
    let runtime_plan = state
        .execution
        .harness
        .prepare(workspace, worktree, &effective, &state.core.daemon_url)
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!("session auth runtime preparation failed: {err:#}"))
        })?;
    let runtime_data_root = runtime_data_root(&runtime_plan.env_overrides);
    Ok(PreparedWorkspaceProbeRuntime {
        cwd,
        runtime_data_root,
        env_overrides: runtime_plan.env_overrides,
    })
}
