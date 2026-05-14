use super::*;
use ctx_workspace_services::workspace_registration::{
    prepare_workspace_registration, WorkspaceRegistrationError,
};

#[derive(Debug, Deserialize)]
pub(in crate::api) struct CreateWorkspaceReq {
    root_path: String,
    name: Option<String>,
}

pub(in crate::api) async fn create_workspace(
    State(workspaces): State<WorkspacesHandle>,
    Json(req): Json<CreateWorkspaceReq>,
) -> Result<Json<Workspace>, (StatusCode, Json<ApiErrorResp>)> {
    let candidate = prepare_workspace_registration(&req.root_path)
        .await
        .map_err(workspace_registration_error_response)?;

    let root_path_str = candidate.root_path.to_string_lossy().to_string();
    let name = req.name.unwrap_or(candidate.default_name);
    let workspace = workspaces
        .create_workspace(name, root_path_str, candidate.vcs_kind)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    workspaces
        .update_workspace_primary_branch(workspace.id, &candidate.primary_branch)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    workspaces.record_workspace_registered().await;
    Ok(Json(workspace))
}

fn workspace_registration_error_response(
    error: WorkspaceRegistrationError,
) -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: logs::redact_sensitive(error.message()),
        }),
    )
}
