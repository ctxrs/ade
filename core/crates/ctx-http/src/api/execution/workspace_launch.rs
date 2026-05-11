use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::WorkspaceId;
use ctx_observability::logs;

use super::super::errors::ApiErrorResp;
use super::super::shared::map_effective_execution_settings_error;
use crate::daemon::{execution_effective, AppState};

pub(super) async fn resolve_workspace_launch_inputs(
    state: &Arc<AppState>,
    raw_workspace_id: Option<&str>,
) -> Result<
    (
        ctx_core::models::Workspace,
        ctx_settings_model::ExecutionSettings,
    ),
    (StatusCode, Json<ApiErrorResp>),
> {
    let Some(raw_workspace_id) = raw_workspace_id else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "workspace_id is required for workspace_launch".to_string(),
            }),
        ));
    };
    let workspace_id = parse_workspace_id(raw_workspace_id)?;
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let execution_settings =
        execution_effective::effective_execution_settings_classified(state.as_ref(), workspace_id)
            .await
            .map_err(map_effective_execution_settings_error)?;

    Ok((workspace, execution_settings))
}

fn parse_workspace_id(
    raw_workspace_id: &str,
) -> Result<WorkspaceId, (StatusCode, Json<ApiErrorResp>)> {
    uuid::Uuid::parse_str(raw_workspace_id.trim())
        .map(WorkspaceId)
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid workspace id".to_string(),
                }),
            )
        })
}
