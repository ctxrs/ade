use std::sync::Arc;

use crate::daemon::execution_effective;
use crate::daemon::DaemonState;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::TerminalSession;
use ctx_settings_model::ExecutionMode;
use ctx_transport_runtime::terminal_launch::{container_terminal_env, TerminalLaunchError};
use ctx_transport_runtime::terminals::TerminalCreateRequest;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;
use ctx_worktree_data_plane::{apply_data_plane_to_execution_settings, workspace_data_plane};

mod container;
mod paths;
mod worktree;

use self::container::prepare_terminal_container_launch;
use self::paths::resolve_terminal_paths;
#[cfg(test)]
pub use self::worktree::infer_terminal_worktree;
use self::worktree::resolve_terminal_worktree;

pub struct CreateTerminalLaunchRequest {
    pub workspace_id: WorkspaceId,
    pub task_id: Option<TaskId>,
    pub session_id: Option<SessionId>,
    pub worktree_id: Option<WorktreeId>,
    pub cwd: Option<String>,
    pub shell: Option<String>,
}

pub async fn create_workspace_terminal(
    state: &Arc<DaemonState>,
    req: CreateTerminalLaunchRequest,
) -> Result<TerminalSession, TerminalLaunchError> {
    let workspace_id = req.workspace_id;
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| internal_error("failed to load workspace"))?
        .ok_or_else(|| not_found("workspace not found"))?;

    let effective = execution_effective::effective_execution_settings(state, workspace_id)
        .await
        .map_err(|_| internal_error("failed to load execution settings"))?;
    let worktree = resolve_terminal_worktree(
        state,
        workspace_id,
        req.worktree_id,
        req.session_id,
        req.task_id,
    )
    .await?;
    let worktree_data_plane = if let Some(worktree) = worktree.as_ref() {
        Some(
            resolve_worktree_data_plane(state.as_ref(), worktree)
                .await
                .map_err(|_| internal_error("failed to resolve worktree data plane"))?,
        )
    } else if matches!(effective.mode, ExecutionMode::Sandbox) {
        Some(workspace_data_plane(&workspace, effective.mode.clone()))
    } else {
        None
    };
    let effective = worktree_data_plane
        .as_ref()
        .map(|data_plane| {
            apply_data_plane_to_execution_settings(&effective, data_plane)
                .map_err(|_| internal_error("failed to apply worktree data plane"))
        })
        .transpose()?
        .unwrap_or(effective);
    let container_mode = matches!(effective.mode, ExecutionMode::Sandbox);
    let paths = resolve_terminal_paths(
        &workspace,
        worktree.as_ref(),
        worktree_data_plane.as_ref(),
        container_mode,
        req.cwd.as_deref(),
        req.shell.as_deref(),
    )
    .await?;

    let (cwd, native_container, shared_vm_container) = prepare_terminal_container_launch(
        state,
        &workspace,
        worktree.as_ref(),
        &effective,
        workspace_id,
        &paths.cwd,
        paths.container_cwd_authority_root.as_deref(),
    )
    .await?;
    let session = state
        .transport
        .terminals
        .create(TerminalCreateRequest {
            workspace_id,
            task_id: req.task_id,
            session_id: req.session_id,
            worktree_id: worktree.as_ref().map(|wt| wt.id),
            cwd,
            shell: paths.shell,
            cols: None,
            rows: None,
            env: if container_mode {
                container_terminal_env()
            } else {
                Default::default()
            },
            native_container,
            shared_vm_container,
        })
        .await
        .map_err(|e| internal_error(format!("failed to create terminal: {e}")))?;

    Ok(session.snapshot())
}

pub(super) fn not_found(error: impl Into<String>) -> TerminalLaunchError {
    TerminalLaunchError::not_found(error)
}

pub(super) fn internal_error(error: impl Into<String>) -> TerminalLaunchError {
    TerminalLaunchError::internal(error)
}

#[cfg(test)]
mod tests;
