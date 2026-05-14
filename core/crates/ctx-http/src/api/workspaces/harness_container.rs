use super::*;
use ctx_daemon::daemon::workspaces::WorkspaceHarnessContainerError;

pub(in crate::api) async fn get_workspace_harness_container(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<Option<ctx_workspace_container::WorkspaceContainerStatus>>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let status = workspaces
        .workspace_harness_container_status(workspace_id)
        .await
        .map_err(workspace_harness_container_status)?;
    Ok(Json(status))
}

pub(in crate::api) async fn stop_workspace_harness_container(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    workspaces
        .stop_workspace_harness_container(workspace_id)
        .await
        .map_err(workspace_harness_container_status)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(in crate::api) async fn ensure_workspace_harness_container(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    workspaces
        .ensure_workspace_harness_container(workspace_id)
        .await
        .map_err(workspace_harness_container_api_error)?;

    Ok(StatusCode::NO_CONTENT)
}

fn workspace_harness_container_status(error: WorkspaceHarnessContainerError) -> StatusCode {
    match error {
        WorkspaceHarnessContainerError::NotFound => StatusCode::NOT_FOUND,
        WorkspaceHarnessContainerError::Internal(_)
        | WorkspaceHarnessContainerError::ExecutionSettings(_)
        | WorkspaceHarnessContainerError::Ensure(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn workspace_harness_container_api_error(
    error: WorkspaceHarnessContainerError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        WorkspaceHarnessContainerError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ),
        WorkspaceHarnessContainerError::Internal(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&err.to_string()),
            }),
        ),
        WorkspaceHarnessContainerError::ExecutionSettings(err) => {
            map_effective_execution_settings_error(err)
        }
        WorkspaceHarnessContainerError::Ensure(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&err.to_string()),
            }),
        ),
    }
}
