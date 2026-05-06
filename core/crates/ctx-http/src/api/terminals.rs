use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;

use super::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::terminal_launch::{
    CreateTerminalLaunchRequest, TerminalLaunchError, TerminalLaunchErrorKind,
};
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
    Ok(Json(state.transport.terminals.list(workspace_id).await))
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

    let session = crate::terminal_launch::create_workspace_terminal(
        &state,
        CreateTerminalLaunchRequest {
            workspace_id,
            task_id,
            session_id,
            worktree_id,
            cwd: req.cwd,
            shell: req.shell,
        },
    )
    .await
    .map_err(terminal_launch_error_response)?;

    Ok(Json(session))
}

fn terminal_launch_error_response(error: TerminalLaunchError) -> (StatusCode, Json<ApiErrorResp>) {
    let status = match error.kind() {
        TerminalLaunchErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        TerminalLaunchErrorKind::NotFound => StatusCode::NOT_FOUND,
        TerminalLaunchErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ApiErrorResp {
            error: error.message().to_string(),
        }),
    )
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

#[derive(Debug, Serialize)]
pub(super) struct TerminalStreamConnectInfo {
    stream_path: String,
    expires_at: DateTime<Utc>,
}

pub(super) async fn mint_terminal_stream_token(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<TerminalStreamConnectInfo>, StatusCode> {
    let terminal_id = TerminalId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let handle = state
        .transport
        .terminals
        .get(terminal_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let (stream_path, expires_at) = handle.issue_stream_connect_path();
    Ok(Json(TerminalStreamConnectInfo {
        stream_path,
        expires_at,
    }))
}
