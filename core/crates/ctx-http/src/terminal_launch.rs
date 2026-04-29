use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;
use ctx_sandbox_container_runtime::sandbox_cli_invocation;

use crate::api::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::settings::{ContainerRuntimeKind, ExecutionMode};
use crate::terminals::{
    NativeContainerTerminalSpec, SharedVmContainerTerminalSpec, TerminalCreateRequest,
};
use crate::worktree_data_plane::resolve_worktree_data_plane;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{TerminalSession, Workspace, Worktree};
use ctx_worktree_data_plane::{
    apply_data_plane_to_execution_settings, workspace_data_plane, WorktreeDataPlane,
};

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
) -> Result<TerminalSession, (StatusCode, Json<ApiErrorResp>)> {
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
            resolve_worktree_data_plane(state, worktree)
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
    let cwd = if container_mode {
        resolve_container_terminal_cwd(
            worktree_data_plane.as_ref().ok_or_else(|| {
                internal_error("sandbox terminal requires a resolved worktree data plane")
            })?,
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
            .unwrap_or_else(default_shell)
    };

    let (native_container, shared_vm_container) = prepare_terminal_container_launch(
        state,
        &workspace,
        worktree.as_ref(),
        &effective,
        workspace_id,
        &cwd,
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
                HashMap::new()
            },
            native_container,
            shared_vm_container,
        })
        .await
        .map_err(|e| internal_error(format!("failed to create terminal: {e}")))?;

    Ok(session.snapshot())
}

async fn prepare_terminal_container_launch(
    state: &Arc<AppState>,
    workspace: &Workspace,
    worktree: Option<&Worktree>,
    effective: &crate::settings::ExecutionSettings,
    workspace_id: WorkspaceId,
    cwd: &FsPath,
) -> Result<
    (
        Option<NativeContainerTerminalSpec>,
        Option<SharedVmContainerTerminalSpec>,
    ),
    (StatusCode, Json<ApiErrorResp>),
> {
    if !matches!(effective.mode, ExecutionMode::Sandbox) {
        return Ok((None, None));
    }

    match effective.container.runtime {
        ContainerRuntimeKind::NativeContainer => {
            state
                .execution
                .harness
                .ensure_workspace_container(workspace, effective, &state.core.daemon_url)
                .await
                .map_err(|e| internal_error(format!("failed to ensure harness container: {e}")))?;
            if worktree.is_none() {
                ensure_materialized_workspace_root(state, workspace).await?;
            }
            let inv = sandbox_cli_invocation(&state.core.data_root)
                .map_err(|e| internal_error(format!("sandbox container CLI unavailable: {e}")))?;
            Ok((
                Some(NativeContainerTerminalSpec {
                    cli_bin: inv.bin,
                    cli_env: inv.env,
                    container_name: ctx_workspace_container::workspace_container_name(workspace_id),
                    workdir: cwd.to_string_lossy().to_string(),
                    user: Some(ctx_workspace_container::CONTAINER_TERMINAL_USER.to_string()),
                }),
                None,
            ))
        }
        ContainerRuntimeKind::SharedVmContainer => {
            if let Some(worktree) = worktree {
                state
                    .execution
                    .harness
                    .ensure_workspace_container_for_worktree(
                        workspace,
                        worktree,
                        effective,
                        &state.core.daemon_url,
                    )
                    .await
                    .map_err(|e| {
                        internal_error(format!("failed to ensure sandbox container: {e}"))
                    })?;
            } else {
                state
                    .execution
                    .harness
                    .ensure_workspace_container(workspace, effective, &state.core.daemon_url)
                    .await
                    .map_err(|e| {
                        internal_error(format!("failed to ensure sandbox container: {e}"))
                    })?;
            }
            if worktree.is_none() {
                ensure_materialized_workspace_root(state, workspace).await?;
            }
            let helper_path = ctx_avf_linux_runtime::helper_path()
                .map_err(|e| internal_error(format!("AVF helper unavailable: {e}")))?;
            Ok((
                None,
                Some(SharedVmContainerTerminalSpec {
                    helper_path,
                    data_root: state.core.data_root.clone(),
                    workspace_id,
                    workdir: cwd.to_string_lossy().to_string(),
                    user: Some(ctx_workspace_container::CONTAINER_TERMINAL_USER.to_string()),
                }),
            ))
        }
    }
}

async fn ensure_materialized_workspace_root(
    state: &Arc<AppState>,
    workspace: &Workspace,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let sandbox_mode = ctx_harness_runtime::selected_sandbox_command_mode(&state.core.data_root)
        .map_err(|err| internal_error(err.to_string()))?;
    ctx_sandbox_materialization::ensure_workspace_root_from_host_copy(
        &state.core.data_root,
        &sandbox_mode,
        workspace,
    )
    .await
    .map_err(|e| internal_error(format!("failed to materialize sandbox workspace root: {e}")))?;
    Ok(())
}

pub(crate) fn default_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

pub(crate) fn container_terminal_env() -> HashMap<String, String> {
    HashMap::from([
        (
            "HOME".to_string(),
            ctx_workspace_container::CONTAINER_TERMINAL_HOME.to_string(),
        ),
        (
            "USER".to_string(),
            ctx_workspace_container::CONTAINER_TERMINAL_USER.to_string(),
        ),
        (
            "LOGNAME".to_string(),
            ctx_workspace_container::CONTAINER_TERMINAL_USER.to_string(),
        ),
    ])
}

pub(crate) async fn infer_terminal_worktree(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: Option<SessionId>,
    task_id: Option<TaskId>,
) -> Result<Option<Worktree>, (StatusCode, Json<ApiErrorResp>)> {
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

pub(crate) fn resolve_container_terminal_cwd(
    data_plane: &WorktreeDataPlane,
    host_workspace_root: &FsPath,
    host_worktree_root: Option<&FsPath>,
    requested_cwd: Option<&FsPath>,
) -> Result<PathBuf, (StatusCode, Json<ApiErrorResp>)> {
    let live_root = if host_worktree_root.is_some() {
        &data_plane.live_worktree_root
    } else {
        &data_plane.live_workspace_root
    };
    let host_root = host_worktree_root.unwrap_or(host_workspace_root);

    let Some(requested) = requested_cwd else {
        return Ok(live_root.clone());
    };

    let requested_str = requested.to_string_lossy().to_string();
    if requested.is_relative() {
        return resolve_path_lexical_within_root(live_root, &requested_str)
            .map_err(|_| bad_request("cwd must be within the container worktree/workspace root"));
    }

    if let Ok(cwd) = resolve_path_lexical_within_root(live_root, &requested_str) {
        return Ok(cwd);
    }

    if let Ok(host_cwd) = resolve_path_lexical_within_root(host_root, &requested_str) {
        let relative = host_cwd
            .strip_prefix(host_root)
            .map_err(|_| bad_request("cwd must be within the container worktree/workspace root"))?;
        return Ok(live_root.join(relative));
    }

    Err(bad_request(
        "cwd must be within the container worktree/workspace root",
    ))
}

pub(crate) async fn resolve_host_terminal_cwd(
    bound_root: &FsPath,
    requested_cwd: Option<&FsPath>,
) -> Result<PathBuf, (StatusCode, Json<ApiErrorResp>)> {
    let candidate = requested_cwd
        .map(|requested| {
            if requested.is_relative() {
                bound_root.join(requested)
            } else {
                requested.to_path_buf()
            }
        })
        .unwrap_or_else(|| bound_root.to_path_buf());
    let cwd = tokio::fs::canonicalize(&candidate)
        .await
        .map_err(|_| bad_request("cwd does not exist"))?;
    if !cwd.starts_with(bound_root) {
        return Err(bad_request("cwd must be within the terminal root"));
    }
    Ok(cwd)
}

pub(crate) async fn resolve_terminal_host_root(
    path: &FsPath,
    container_mode: bool,
    unavailable_error: &'static str,
) -> Result<PathBuf, (StatusCode, Json<ApiErrorResp>)> {
    if container_mode {
        return Ok(path.to_path_buf());
    }

    tokio::fs::canonicalize(path)
        .await
        .map_err(|_| bad_request(unavailable_error))
}

fn error_response(
    status: StatusCode,
    error: impl Into<String>,
) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

fn bad_request(error: impl Into<String>) -> (StatusCode, Json<ApiErrorResp>) {
    error_response(StatusCode::BAD_REQUEST, error)
}

fn not_found(error: impl Into<String>) -> (StatusCode, Json<ApiErrorResp>) {
    error_response(StatusCode::NOT_FOUND, error)
}

fn internal_error(error: impl Into<String>) -> (StatusCode, Json<ApiErrorResp>) {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, error)
}

fn resolve_path_lexical_within_root(root: &FsPath, path: &str) -> anyhow::Result<PathBuf> {
    let candidate = if PathBuf::from(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    };
    let mut is_abs = false;
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for comp in candidate.components() {
        use std::path::Component;
        match comp {
            Component::Prefix(_) => anyhow::bail!("unsupported path prefix"),
            Component::RootDir => {
                is_abs = true;
                parts.clear();
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.is_empty() {
                    continue;
                }
                parts.pop();
            }
            Component::Normal(seg) => parts.push(seg.to_os_string()),
        }
    }
    let mut normalized = PathBuf::new();
    if is_abs {
        normalized.push(std::path::MAIN_SEPARATOR.to_string());
    }
    for part in &parts {
        normalized.push(part);
    }
    if !normalized.starts_with(root) {
        anyhow::bail!("path outside root");
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests;
