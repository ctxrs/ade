use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::api::errors::ApiErrorResp;
use crate::daemon::terminals::CreateTerminalLaunchRequest;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};

#[derive(Debug, Deserialize)]
pub(in crate::api) struct CreateTerminalReq {
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

pub(super) fn parse_create_terminal_launch_request(
    raw_workspace_id: &str,
    req: CreateTerminalReq,
) -> Result<CreateTerminalLaunchRequest, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = WorkspaceId(
        uuid::Uuid::parse_str(raw_workspace_id).map_err(|_| bad_request("invalid workspace id"))?,
    );
    let task_id = parse_optional_id(req.task_id, "invalid task_id")?.map(TaskId);
    let session_id = parse_optional_id(req.session_id, "invalid session_id")?.map(SessionId);
    let worktree_id = parse_optional_id(req.worktree_id, "invalid worktree_id")?.map(WorktreeId);

    Ok(CreateTerminalLaunchRequest {
        workspace_id,
        task_id,
        session_id,
        worktree_id,
        cwd: req.cwd,
        shell: req.shell,
    })
}

fn parse_optional_id(
    raw: Option<String>,
    error: &'static str,
) -> Result<Option<uuid::Uuid>, (StatusCode, Json<ApiErrorResp>)> {
    raw.map(|value| uuid::Uuid::parse_str(value.trim()).map_err(|_| bad_request(error)))
        .transpose()
}

fn bad_request(error: &'static str) -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: error.to_string(),
        }),
    )
}
