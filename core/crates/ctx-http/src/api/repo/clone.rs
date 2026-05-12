use super::*;
use ctx_workspace_services::repo_onboarding::RepoCloneRequest;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct RepoCloneReq {
    repo_url: String,
    dest_parent: String,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    dest_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct RepoCloneResp {
    path: String,
}

pub(in crate::api) async fn repo_clone(
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<RepoCloneReq>,
) -> Result<Json<RepoCloneResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let path = ctx_workspace_services::repo_onboarding::clone_repo(RepoCloneRequest {
        repo_url: &req.repo_url,
        dest_parent: &req.dest_parent,
        branch: req.branch.as_deref(),
        dest_name: req.dest_name.as_deref(),
    })
    .await
    .map_err(repo_onboarding_workflow_error_response)?;

    Ok(Json(RepoCloneResp {
        path: path.to_string_lossy().to_string(),
    }))
}
