use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use super::errors::ApiErrorResp;
use crate::buffers::BufferStore;
use crate::container_fs::is_container_path;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::harness_runtime;
use crate::settings::{ContainerMountMode, ContainerRuntimeKind, ExecutionMode};
use crate::terminals::{AvfLinuxTerminalSpec, PodmanTerminalSpec, TerminalCreateRequest};
use ctx_core::ids::{SessionId, TaskId, TerminalId, WorkspaceId, WorktreeId};
use ctx_core::models::TerminalSession;

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

async fn infer_avf_terminal_worktree(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: Option<SessionId>,
    task_id: Option<TaskId>,
) -> Option<ctx_core::models::Worktree> {
    if let Some(session_id) = session_id {
        if let Ok(store) = state.store_for_session(session_id).await {
            if let Ok(Some(session)) = store.get_session(session_id).await {
                if let Ok(Some(worktree)) = store.get_worktree(session.worktree_id).await {
                    return Some(worktree);
                }
            }
        }
    }

    if let Some(task_id) = task_id {
        if let Ok(store) = state.store_for_task(task_id).await {
            if let Ok(Some(task)) = store.get_task(task_id).await {
                if let Some(primary_worktree_id) = task.primary_worktree_id {
                    if let Ok(Some(worktree)) = store.get_worktree(primary_worktree_id).await {
                        return Some(worktree);
                    }
                }
            }
        }
    }

    if let Ok(store) = state.store_for_workspace(workspace_id).await {
        if let Ok(worktrees) = store.list_worktrees(workspace_id).await {
            return worktrees.into_iter().last();
        }
    }

    None
}

fn avf_guest_worktree_root(worktree_id: WorktreeId) -> PathBuf {
    PathBuf::from(harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT)
        .join("worktrees")
        .join(worktree_id.0.to_string())
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

    let workspace_root = PathBuf::from(&workspace.root_path);
    let workspace_root = tokio::fs::canonicalize(&workspace_root)
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "workspace root is unavailable".to_string(),
                }),
            )
        })?;

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

    let container_mode = matches!(effective.mode, ExecutionMode::Container);
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
        Some(wt)
    } else if container_mode
        && matches!(
            effective.container.runtime,
            ContainerRuntimeKind::AvfLinuxVm
        )
    {
        infer_avf_terminal_worktree(&state, workspace_id, session_id, task_id).await
    } else {
        None
    };

    let worktree_root = if let Some(wt) = worktree.as_ref() {
        let root = PathBuf::from(&wt.root_path);
        if is_container_path(&root) {
            Some(root)
        } else {
            Some(tokio::fs::canonicalize(&root).await.map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "worktree root is unavailable".to_string(),
                    }),
                )
            })?)
        }
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
    let container_mode = container_mode
        || worktree_root
            .as_ref()
            .map(|root| is_container_path(root))
            .unwrap_or(false);
    let avf_guest_worktree_root = if container_mode
        && matches!(
            effective.container.runtime,
            ContainerRuntimeKind::AvfLinuxVm
        ) {
        worktree.as_ref().map(|wt| avf_guest_worktree_root(wt.id))
    } else {
        None
    };
    let container_workspace_root = match effective.container.mount_mode {
        ContainerMountMode::DiskIsolated => {
            PathBuf::from(harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT)
        }
        // Host-mounted uses host paths directly inside the container.
        ContainerMountMode::HostMounted => workspace_root.clone(),
    };

    let cwd = if container_mode {
        let fallback = avf_guest_worktree_root
            .clone()
            .or_else(|| worktree_root.clone())
            .unwrap_or_else(|| container_workspace_root.clone());
        if let Some(requested) = requested_cwd.as_ref() {
            let requested_str = requested.to_string_lossy().to_string();
            let resolved = if let (Some(host_root), Some(guest_root)) =
                (worktree_root.as_ref(), avf_guest_worktree_root.as_ref())
            {
                BufferStore::resolve_path_lexical(host_root, &requested_str)
                    .ok()
                    .and_then(|resolved_host| {
                        resolved_host.strip_prefix(host_root).ok().map(|suffix| {
                            if suffix.as_os_str().is_empty() {
                                guest_root.clone()
                            } else {
                                guest_root.join(suffix)
                            }
                        })
                    })
                    .or_else(|| BufferStore::resolve_path_lexical(guest_root, &requested_str).ok())
                    .or_else(|| {
                        BufferStore::resolve_path_lexical(&container_workspace_root, &requested_str)
                            .ok()
                    })
            } else {
                // Allow cwd within either the worktree root or the container workspace root.
                worktree_root
                    .as_ref()
                    .and_then(|root| BufferStore::resolve_path_lexical(root, &requested_str).ok())
                    .or_else(|| {
                        BufferStore::resolve_path_lexical(&container_workspace_root, &requested_str)
                            .ok()
                    })
            }
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "cwd must be within the container worktree/workspace root".to_string(),
                }),
            ))?;
            resolved
        } else {
            fallback
        }
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

    let (podman, avf_linux_vm) = if container_mode {
        match effective.container.runtime {
            ContainerRuntimeKind::Podman => {
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
                let inv =
                    harness_runtime::podman_invocation(&state.core.data_root).map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiErrorResp {
                                error: format!("podman unavailable: {e}"),
                            }),
                        )
                    })?;
                (
                    Some(PodmanTerminalSpec {
                        podman_bin: inv.bin,
                        podman_env: inv.env,
                        container_name: harness_runtime::workspace_container_name(workspace_id),
                        workdir: cwd.to_string_lossy().to_string(),
                    }),
                    None,
                )
            }
            ContainerRuntimeKind::AvfLinuxVm => {
                let worktree = worktree.as_ref().ok_or((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "sandbox terminals require a worktree for the AVF runtime"
                            .to_string(),
                    }),
                ))?;
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
                                error: format!("failed to ensure AVF workspace VM: {e}"),
                            }),
                        )
                    })?;
                let helper_path = harness_runtime::avf_linux_helper_path().map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: format!("AVF helper unavailable: {e}"),
                        }),
                    )
                })?;
                (
                    None,
                    Some(AvfLinuxTerminalSpec {
                        helper_path,
                        data_root: state.core.data_root.clone(),
                        workspace_id,
                        worktree_id: worktree.id,
                        workdir: cwd.to_string_lossy().to_string(),
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
            env: std::collections::HashMap::new(),
            podman,
            avf_linux_vm,
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
