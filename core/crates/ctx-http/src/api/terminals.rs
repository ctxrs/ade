use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use super::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::settings::{self, NetworkContext};
use crate::terminals::TerminalCreateRequest;
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

    let worktree_root = if let Some(wt_id) = worktree_id {
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
        Some(tokio::fs::canonicalize(&wt.root_path).await.map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "worktree root is unavailable".to_string(),
                }),
            )
        })?)
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

    let requested_shell = req.shell.as_deref().and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let shell = requested_shell
        .map(|value| value.to_string())
        .unwrap_or_else(default_shell);
    let settings = settings::load_settings(&state.core.data_root).await;
    let network_profiles = settings.network_profiles.unwrap_or_default();
    let network_profile = network_profiles.profile(NetworkContext::UserShell);
    let proxy_env = state
        .execution
        .egress_proxy
        .proxy_env_for_context(
            workspace_id,
            NetworkContext::UserShell,
            network_profile,
            "127.0.0.1",
        )
        .await;
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
            env: proxy_env,
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
