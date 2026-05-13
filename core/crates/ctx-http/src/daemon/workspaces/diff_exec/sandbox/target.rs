use std::sync::Arc;

use ctx_core::models::Worktree;
use ctx_settings_model::ContainerRuntimeKind;
use ctx_workspace_container::workspace_container_name;
use ctx_worktree_data_plane::{
    apply_data_plane_to_execution_settings,
    resolve_worktree_data_plane_with_host as resolve_worktree_data_plane,
};

use crate::daemon::{execution_effective, AppState};

pub(super) enum SandboxExecTarget {
    NativeContainer { container_name: String },
    SharedVmContainer,
}

pub(super) async fn ensure_container_for_worktree(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> anyhow::Result<SandboxExecTarget> {
    let data_plane = resolve_worktree_data_plane(state.as_ref(), worktree).await?;
    let effective =
        execution_effective::effective_execution_settings(state, data_plane.workspace.id).await?;
    let effective = apply_data_plane_to_execution_settings(&effective, &data_plane)?;
    state
        .execution
        .harness
        .ensure_workspace_container_for_worktree(
            &data_plane.workspace,
            worktree,
            &effective,
            &state.core.daemon_url,
        )
        .await?;
    if matches!(
        effective.container.runtime,
        ContainerRuntimeKind::SharedVmContainer
    ) {
        Ok(SandboxExecTarget::SharedVmContainer)
    } else {
        Ok(SandboxExecTarget::NativeContainer {
            container_name: workspace_container_name(data_plane.workspace.id),
        })
    }
}
