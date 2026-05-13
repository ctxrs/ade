use super::*;
use crate::daemon::workspaces::{self, WorkspaceDeleteError};

pub(in crate::api) async fn delete_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    workspaces::delete_workspace(&state, id)
        .await
        .map_err(delete_workspace_error_status)?;
    Ok(StatusCode::NO_CONTENT)
}

fn delete_workspace_error_status(error: WorkspaceDeleteError) -> StatusCode {
    match error {
        WorkspaceDeleteError::NotFound => StatusCode::NOT_FOUND,
        WorkspaceDeleteError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
