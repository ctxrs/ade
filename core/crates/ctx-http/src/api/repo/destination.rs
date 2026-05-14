use super::*;
use crate::daemon::CoreHandle;
use ctx_workspace_services::repo_onboarding::RepoValidateDestinationRequest;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct RepoValidateDestinationReq {
    path: String,
    #[serde(default)]
    must_not_exist: bool,
    #[serde(default)]
    require_empty_if_exists: bool,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct RepoValidateDestinationResp {
    path: String,
}

pub(in crate::api) async fn repo_validate_destination(
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<RepoValidateDestinationReq>,
) -> Result<Json<RepoValidateDestinationResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    validate_destination(req).await
}

pub(in crate::api) async fn repo_validate_destination_get(
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Query(req): Query<RepoValidateDestinationReq>,
) -> Result<Json<RepoValidateDestinationResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    validate_destination(req).await
}

async fn validate_destination(
    req: RepoValidateDestinationReq,
) -> Result<Json<RepoValidateDestinationResp>, (StatusCode, Json<ApiErrorResp>)> {
    let path = ctx_workspace_services::repo_onboarding::validate_repo_destination(
        RepoValidateDestinationRequest {
            path: &req.path,
            must_not_exist: req.must_not_exist,
            require_empty_if_exists: req.require_empty_if_exists,
        },
    )
    .await
    .map_err(repo_onboarding_path_error_response)?;

    Ok(Json(RepoValidateDestinationResp {
        path: path.to_string_lossy().to_string(),
    }))
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct RepoStagingPathResp {
    path: String,
}

/// Returns a unique staging path under data_root/workspaces/staging/<uuid>.
/// Used for disk-isolated clone/new: the daemon manages the path so the wizard
/// doesn't need to ask the user for a host destination.
pub(in crate::api) async fn repo_staging_path(
    mobile_auth: Option<Extension<MobileAuthContext>>,
    State(state): State<CoreHandle>,
) -> Result<Json<RepoStagingPathResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let path = ctx_workspace_services::repo_onboarding::create_repo_staging_path(state.data_root())
        .await
        .map_err(repo_staging_path_error_response)?;

    Ok(Json(RepoStagingPathResp {
        path: path.to_string_lossy().to_string(),
    }))
}
