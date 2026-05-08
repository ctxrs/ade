use std::path::PathBuf;
use std::sync::Arc;

use crate::daemon::AppState;
use crate::execution_effective;
use crate::settings::ExecutionMode;
use crate::terminals::TerminalCreateRequest;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{TerminalSession, Worktree};
use ctx_transport_runtime::terminal_launch::{
    container_terminal_env, default_terminal_shell, resolve_container_terminal_cwd,
    resolve_host_terminal_cwd, resolve_terminal_host_root, TerminalLaunchError,
};
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;
use ctx_worktree_data_plane::{apply_data_plane_to_execution_settings, workspace_data_plane};

mod container;

use self::container::prepare_terminal_container_launch;

pub(crate) struct CreateTerminalLaunchRequest {
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) task_id: Option<TaskId>,
    pub(crate) session_id: Option<SessionId>,
    pub(crate) worktree_id: Option<WorktreeId>,
    pub(crate) cwd: Option<String>,
    pub(crate) shell: Option<String>,
}

pub(crate) async fn create_workspace_terminal(
    state: &Arc<AppState>,
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
    let worktree = if let Some(wt_id) = req.worktree_id {
        let store = state
            .store_for_worktree(wt_id)
            .await
            .map_err(|_| not_found("worktree not found"))?;
        let wt = store
            .get_worktree(wt_id)
            .await
            .map_err(|_| internal_error("failed to load worktree"))?
            .ok_or_else(|| not_found("worktree not found"))?;
        if wt.workspace_id != workspace_id {
            return Err(not_found("worktree not found"));
        }
        Some(wt)
    } else if req.session_id.is_some() || req.task_id.is_some() {
        infer_terminal_worktree(state, workspace_id, req.session_id, req.task_id).await?
    } else {
        None
    };
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
    let workspace_root_path = PathBuf::from(&workspace.root_path);
    let workspace_root: PathBuf = resolve_terminal_host_root(
        &workspace_root_path,
        container_mode,
        "workspace root is unavailable",
    )
    .await?;

    let worktree_root: Option<PathBuf> = if let Some(wt) = worktree.as_ref() {
        let root = PathBuf::from(&wt.root_path);
        Some(
            resolve_terminal_host_root(&root, container_mode, "worktree root is unavailable")
                .await?,
        )
    } else {
        None
    };

    let requested_cwd = req.cwd.as_ref().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        }
    });
    let container_cwd_authority_root = if container_mode {
        Some(if worktree_root.is_some() {
            worktree_data_plane
                .as_ref()
                .ok_or_else(|| {
                    internal_error("sandbox terminal requires a resolved worktree data plane")
                })?
                .live_worktree_root
                .clone()
        } else {
            worktree_data_plane
                .as_ref()
                .ok_or_else(|| {
                    internal_error("sandbox terminal requires a resolved worktree data plane")
                })?
                .live_workspace_root
                .clone()
        })
    } else {
        None
    };
    let cwd = if container_mode {
        let data_plane = worktree_data_plane.as_ref().ok_or_else(|| {
            internal_error("sandbox terminal requires a resolved worktree data plane")
        })?;
        resolve_container_terminal_cwd(
            &data_plane.live_workspace_root,
            worktree_root
                .as_ref()
                .map(|_| data_plane.live_worktree_root.as_path()),
            &workspace_root,
            worktree_root.as_deref(),
            requested_cwd.as_deref(),
        )?
    } else {
        let bound_root = worktree_root.as_deref().unwrap_or(&workspace_root);
        resolve_host_terminal_cwd(bound_root, requested_cwd.as_deref()).await?
    };

    let requested_shell = req.shell.as_deref().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let shell = if container_mode {
        requested_shell
            .map(ToString::to_string)
            .unwrap_or_else(|| "/bin/bash".to_string())
    } else {
        requested_shell
            .map(ToString::to_string)
            .unwrap_or_else(default_terminal_shell)
    };

    let (cwd, native_container, shared_vm_container) = prepare_terminal_container_launch(
        state,
        &workspace,
        worktree.as_ref(),
        &effective,
        workspace_id,
        &cwd,
        container_cwd_authority_root.as_deref(),
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
            shell,
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

pub(crate) async fn infer_terminal_worktree(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: Option<SessionId>,
    task_id: Option<TaskId>,
) -> Result<Option<Worktree>, TerminalLaunchError> {
    if let Some(session_id) = session_id {
        let store = state
            .store_for_session(session_id)
            .await
            .map_err(|_| not_found("session not found"))?;
        let session = store
            .get_session(session_id)
            .await
            .map_err(|_| internal_error("failed to load session"))?
            .ok_or_else(|| not_found("session not found"))?;
        if session.workspace_id != workspace_id {
            return Err(not_found("session not found"));
        }
        let worktree = store
            .get_worktree(session.worktree_id)
            .await
            .map_err(|_| internal_error("failed to load worktree"))?
            .ok_or_else(|| not_found("worktree not found"))?;
        if worktree.workspace_id != workspace_id {
            return Err(not_found("worktree not found"));
        }
        return Ok(Some(worktree));
    }

    if let Some(task_id) = task_id {
        let store = state
            .store_for_task(task_id)
            .await
            .map_err(|_| not_found("task not found"))?;
        let task = store
            .get_task(task_id)
            .await
            .map_err(|_| internal_error("failed to load task"))?
            .ok_or_else(|| not_found("task not found"))?;
        if task.workspace_id != workspace_id {
            return Err(not_found("task not found"));
        }
        let primary_worktree_id = task
            .primary_worktree_id
            .ok_or_else(|| not_found("worktree not found"))?;
        let worktree = store
            .get_worktree(primary_worktree_id)
            .await
            .map_err(|_| internal_error("failed to load worktree"))?
            .ok_or_else(|| not_found("worktree not found"))?;
        if worktree.workspace_id != workspace_id {
            return Err(not_found("worktree not found"));
        }
        return Ok(Some(worktree));
    }

    if let Ok(store) = state.store_for_workspace(workspace_id).await {
        if let Ok(worktrees) = store.list_worktrees(workspace_id).await {
            return Ok(worktrees.into_iter().last());
        }
    }

    Ok(None)
}

fn not_found(error: impl Into<String>) -> TerminalLaunchError {
    TerminalLaunchError::not_found(error)
}

pub(super) fn internal_error(error: impl Into<String>) -> TerminalLaunchError {
    TerminalLaunchError::internal(error)
}

#[cfg(test)]
mod tests;
