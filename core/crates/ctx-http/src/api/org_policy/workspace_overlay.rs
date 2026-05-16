use super::common::{parse_workspace_id, policy_api_error};
use super::*;

pub(in crate::api) async fn get_workspace_org_policy(
    State(state): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<Option<WorkspacePolicyOverlay>>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&id)?;
    state
        .get_workspace_policy_overlay(workspace_id)
        .await
        .map(Json)
        .map_err(|err| workspace_policy_error(err, "failed to load workspace org policy"))
}

pub(in crate::api) async fn upsert_workspace_org_policy(
    State(state): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Json(overlay): Json<WorkspacePolicyOverlay>,
) -> Result<Json<WorkspacePolicyOverlay>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&id)?;
    if overlay.workspace_id != workspace_id {
        return Err(policy_api_error(
            StatusCode::BAD_REQUEST,
            "workspace policy overlay workspace_id must match route workspace id",
        ));
    }
    state
        .upsert_workspace_policy_overlay_checked(overlay)
        .await
        .map(Json)
        .map_err(upsert_workspace_policy_error)
}

fn upsert_workspace_policy_error(
    error: UpsertWorkspacePolicyOverlayError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        UpsertWorkspacePolicyOverlayError::EnrollmentMissing => {
            policy_api_error(StatusCode::CONFLICT, "daemon is not enrolled for this org")
        }
        UpsertWorkspacePolicyOverlayError::EnrollmentLoad(error) => policy_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to load daemon enrollment: {error:#}"),
        ),
        UpsertWorkspacePolicyOverlayError::WorkspaceNotFound => {
            policy_api_error(StatusCode::NOT_FOUND, "workspace not found for org policy")
        }
        UpsertWorkspacePolicyOverlayError::Store(error) => policy_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to upsert workspace org policy: {error:#}"),
        ),
    }
}

fn workspace_policy_error(
    error: WorkspacePolicyOverlayError,
    message: &'static str,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        WorkspacePolicyOverlayError::WorkspaceNotFound => {
            policy_api_error(StatusCode::NOT_FOUND, "workspace not found for org policy")
        }
        WorkspacePolicyOverlayError::Store(error) => policy_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{message}: {error:#}"),
        ),
    }
}
