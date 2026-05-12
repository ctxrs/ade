use super::*;
use ctx_workspace_services::repo_onboarding::RepoInitRequest;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct RepoInitReq {
    path: String,
    #[serde(default)]
    allow_existing: bool,
    #[serde(default)]
    allow_non_empty: bool,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct RepoInitResp {
    path: String,
}

pub(in crate::api) async fn repo_init(
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<RepoInitReq>,
) -> Result<Json<RepoInitResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let path = ctx_workspace_services::repo_onboarding::initialize_repo(RepoInitRequest {
        path: &req.path,
        allow_existing: req.allow_existing,
        allow_non_empty: req.allow_non_empty,
    })
    .await
    .map_err(repo_onboarding_workflow_error_response)?;

    Ok(Json(RepoInitResp {
        path: path.to_string_lossy().to_string(),
    }))
}
