use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use super::errors::ApiErrorResp;
use crate::buffers::BufferStore;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::workspace_runtime;
use crate::settings::{ContainerRuntimeKind, ExecutionMode};
use crate::terminals::{
    NativeContainerTerminalSpec, SharedVmContainerTerminalSpec, TerminalCreateRequest,
};
use crate::worktree_data_plane::{
    apply_data_plane_to_execution_settings, map_host_or_live_path_to_live_path,
    resolve_worktree_data_plane, workspace_data_plane, WorktreeDataPlane,
};
use ctx_core::ids::{SessionId, TaskId, TerminalId, WorkspaceId, WorktreeId};
use ctx_core::models::TerminalSession;

#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize)]
pub(super) struct CreateTerminalReq {
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    shell: Option<String>,
}

pub(super) async fn list_workspace_terminals(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TerminalSession>>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let terminals = state.transport.terminals.list(workspace_id).await;
    Ok(Json(terminals))
}

fn default_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

fn container_terminal_env() -> HashMap<String, String> {
    HashMap::from([
        (
            "HOME".to_string(),
            crate::workspace_runtime::CONTAINER_TERMINAL_HOME.to_string(),
        ),
        (
            "USER".to_string(),
            crate::workspace_runtime::CONTAINER_TERMINAL_USER.to_string(),
        ),
        (
            "LOGNAME".to_string(),
            crate::workspace_runtime::CONTAINER_TERMINAL_USER.to_string(),
        ),
    ])
}

async fn infer_terminal_worktree(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: Option<SessionId>,
    task_id: Option<TaskId>,
) -> Result<Option<ctx_core::models::Worktree>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(session_id) = session_id {
        let store = state.store_for_session(session_id).await.map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            )
        })?;
        let session = store.get_session(session_id).await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?;
        let session = session.ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
        if session.workspace_id != workspace_id {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            ));
        }
        let worktree = store.get_worktree(session.worktree_id).await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?;
        let worktree = worktree.ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
        if worktree.workspace_id != workspace_id {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            ));
        }
        return Ok(Some(worktree));
    }

    if let Some(task_id) = task_id {
        let store = state.store_for_task(task_id).await.map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "task not found".to_string(),
                }),
            )
        })?;
        let task = store.get_task(task_id).await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load task".to_string(),
                }),
            )
        })?;
        let task = task.ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "task not found".to_string(),
            }),
        ))?;
        if task.workspace_id != workspace_id {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "task not found".to_string(),
                }),
            ));
        }
        let primary_worktree_id = task.primary_worktree_id.ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
        let worktree = store.get_worktree(primary_worktree_id).await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load worktree".to_string(),
                }),
            )
        })?;
        let worktree = worktree.ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "worktree not found".to_string(),
            }),
        ))?;
        if worktree.workspace_id != workspace_id {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            ));
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

fn resolve_container_terminal_cwd(
    data_plane: &WorktreeDataPlane,
    host_workspace_root: &FsPath,
    host_worktree_root: Option<&FsPath>,
    requested_cwd: Option<&FsPath>,
) -> Result<PathBuf, (StatusCode, Json<ApiErrorResp>)> {
    let fallback = data_plane.live_worktree_root.clone();

    let Some(requested) = requested_cwd else {
        return Ok(fallback);
    };

    let requested_str = requested.to_string_lossy().to_string();
    if requested.is_relative() {
        return BufferStore::resolve_path_lexical(&fallback, &requested_str).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "cwd must be within the container worktree/workspace root".to_string(),
                }),
            )
        });
    }

    if let Some(mapped) = map_host_or_live_path_to_live_path(
        data_plane,
        host_workspace_root,
        host_worktree_root,
        requested,
    ) {
        return Ok(mapped);
    }

    Err((
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: "cwd must be within the container worktree/workspace root".to_string(),
        }),
    ))
}

async fn resolve_terminal_host_root(
    path: &FsPath,
    container_mode: bool,
    unavailable_error: &'static str,
) -> Result<PathBuf, (StatusCode, Json<ApiErrorResp>)> {
    if container_mode {
        return Ok(path.to_path_buf());
    }

    tokio::fs::canonicalize(path).await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: unavailable_error.to_string(),
            }),
        )
    })
}

pub(super) async fn create_workspace_terminal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateTerminalReq>,
) -> Result<Json<TerminalSession>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);

    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load workspace".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let task_id = match req.task_id {
        Some(raw) => Some(TaskId(uuid::Uuid::parse_str(raw.trim()).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid task_id".to_string(),
                }),
            )
        })?)),
        None => None,
    };
    let session_id = match req.session_id {
        Some(raw) => Some(SessionId(uuid::Uuid::parse_str(raw.trim()).map_err(
            |_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "invalid session_id".to_string(),
                    }),
                )
            },
        )?)),
        None => None,
    };
    let worktree_id = match req.worktree_id {
        Some(raw) => Some(WorktreeId(uuid::Uuid::parse_str(raw.trim()).map_err(
            |_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "invalid worktree_id".to_string(),
                    }),
                )
            },
        )?)),
        None => None,
    };

    let effective = execution_effective::effective_execution_settings(&state, workspace_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load execution settings".to_string(),
                }),
            )
        })?;
    let worktree = if let Some(wt_id) = worktree_id {
        let store = state.store_for_worktree(wt_id).await.map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            )
        })?;
        let wt = store
            .get_worktree(wt_id)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to load worktree".to_string(),
                    }),
                )
            })?
            .ok_or((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            ))?;
        if wt.workspace_id != workspace_id {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            ));
        }
        Some(wt)
    } else if session_id.is_some() || task_id.is_some() {
        infer_terminal_worktree(&state, workspace_id, session_id, task_id).await?
    } else {
        None
    };
    let worktree_data_plane = if let Some(worktree) = worktree.as_ref() {
        Some(
            resolve_worktree_data_plane(&state, worktree)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "failed to resolve worktree data plane".to_string(),
                        }),
                    )
                })?,
        )
    } else if matches!(effective.mode, ExecutionMode::Sandbox) {
        Some(workspace_data_plane(&workspace, effective.mode.clone()))
    } else {
        None
    };
    let effective = worktree_data_plane
        .as_ref()
        .map(|data_plane| {
            apply_data_plane_to_execution_settings(&effective, data_plane).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to apply worktree data plane".to_string(),
                    }),
                )
            })
        })
        .transpose()?
        .unwrap_or(effective);
    let container_mode = matches!(effective.mode, ExecutionMode::Sandbox);
    let workspace_root_path = PathBuf::from(&workspace.root_path);
    let workspace_root = resolve_terminal_host_root(
        &workspace_root_path,
        container_mode,
        "workspace root is unavailable",
    )
    .await?;

    let worktree_root = if let Some(wt) = worktree.as_ref() {
        let root = PathBuf::from(&wt.root_path);
        Some(
            resolve_terminal_host_root(&root, container_mode, "worktree root is unavailable")
                .await?,
        )
    } else {
        None
    };

    let requested_cwd = req.cwd.as_ref().and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        }
    });
    let cwd = if container_mode {
        resolve_container_terminal_cwd(
            worktree_data_plane.as_ref().ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "sandbox terminal requires a resolved worktree data plane".to_string(),
                }),
            ))?,
            &workspace_root,
            worktree_root.as_deref(),
            requested_cwd.as_deref(),
        )?
    } else {
        let fallback_cwd = worktree_root
            .clone()
            .unwrap_or_else(|| workspace_root.clone());
        let cwd = requested_cwd.unwrap_or(fallback_cwd);
        let cwd = tokio::fs::canonicalize(&cwd).await.map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "cwd does not exist".to_string(),
                }),
            )
        })?;
        let allowed = worktree_root
            .as_ref()
            .map(|root| cwd.starts_with(root))
            .unwrap_or(false)
            || cwd.starts_with(&workspace_root);
        if !allowed {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "cwd must be within the workspace or worktree".to_string(),
                }),
            ));
        }
        cwd
    };

    let requested_shell = req.shell.as_deref().and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let shell = if container_mode {
        // The harness container is always Linux; use a deterministic in-container default.
        requested_shell
            .map(|value| value.to_string())
            .unwrap_or_else(|| "/bin/bash".to_string())
    } else {
        requested_shell
            .map(|value| value.to_string())
            .unwrap_or_else(default_shell)
    };

    let (native_container, shared_vm_container) = if container_mode {
        match effective.container.runtime {
            ContainerRuntimeKind::NativeContainer => {
                state
                    .execution
                    .harness
                    .ensure_workspace_container(&workspace, &effective, &state.core.daemon_url)
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiErrorResp {
                                error: format!("failed to ensure harness container: {e}"),
                            }),
                        )
                    })?;
                if worktree.is_none() {
                    crate::disk_isolated::ensure_workspace_root_from_host_copy(
                        &state.core.data_root,
                        &workspace,
                    )
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiErrorResp {
                                error: format!("failed to materialize sandbox workspace root: {e}"),
                            }),
                        )
                    })?;
                }
                let inv = workspace_runtime::sandbox_cli_invocation(&state.core.data_root).map_err(
                    |e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiErrorResp {
                                error: format!("sandbox container CLI unavailable: {e}"),
                            }),
                        )
                    },
                )?;
                (
                    Some(NativeContainerTerminalSpec {
                        cli_bin: inv.bin,
                        cli_env: inv.env,
                        container_name: workspace_runtime::workspace_container_name(workspace_id),
                        workdir: cwd.to_string_lossy().to_string(),
                        user: Some(crate::workspace_runtime::CONTAINER_TERMINAL_USER.to_string()),
                    }),
                    None,
                )
            }
            ContainerRuntimeKind::SharedVmContainer => {
                if let Some(worktree) = worktree.as_ref() {
                    state
                        .execution
                        .harness
                        .ensure_workspace_container_for_worktree(
                            &workspace,
                            worktree,
                            &effective,
                            &state.core.daemon_url,
                        )
                        .await
                        .map_err(|e| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ApiErrorResp {
                                    error: format!("failed to ensure sandbox container: {e}"),
                                }),
                            )
                        })?;
                } else {
                    state
                        .execution
                        .harness
                        .ensure_workspace_container(&workspace, &effective, &state.core.daemon_url)
                        .await
                        .map_err(|e| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ApiErrorResp {
                                    error: format!("failed to ensure sandbox container: {e}"),
                                }),
                            )
                        })?;
                }
                if worktree.is_none() {
                    crate::disk_isolated::ensure_workspace_root_from_host_copy(
                        &state.core.data_root,
                        &workspace,
                    )
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiErrorResp {
                                error: format!("failed to materialize sandbox workspace root: {e}"),
                            }),
                        )
                    })?;
                }
                let helper_path = workspace_runtime::avf_linux_helper_path().map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: format!("AVF helper unavailable: {e}"),
                        }),
                    )
                })?;
                (
                    None,
                    Some(SharedVmContainerTerminalSpec {
                        helper_path,
                        data_root: state.core.data_root.clone(),
                        workspace_id,
                        workdir: cwd.to_string_lossy().to_string(),
                        user: Some(crate::workspace_runtime::CONTAINER_TERMINAL_USER.to_string()),
                    }),
                )
            }
        }
    } else {
        (None, None)
    };
    let session = state
        .transport
        .terminals
        .create(TerminalCreateRequest {
            workspace_id,
            task_id,
            session_id,
            worktree_id,
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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: format!("failed to create terminal: {e}"),
                }),
            )
        })?;

    Ok(Json(session.snapshot()))
}

pub(super) async fn delete_terminal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let terminal_id = TerminalId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let session = state.transport.terminals.remove(terminal_id).await;
    if let Some(session) = session {
        let _ = session.kill();
        session.mark_exited(None);
        return Ok(StatusCode::NO_CONTENT);
    }
    Err(StatusCode::NOT_FOUND)
}
