use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct RepoStatusReq {
    path: String,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct RepoStatusResp {
    canonical_path: String,
    is_repo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub(in crate::api) async fn repo_status(
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<RepoStatusReq>,
) -> Result<Json<RepoStatusResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;

    let status = ctx_workspace_services::repo_onboarding::inspect_repo_status(&req.path)
        .await
        .map_err(repo_onboarding_workflow_error_response)?;
    Ok(Json(RepoStatusResp {
        canonical_path: status.canonical_path.to_string_lossy().to_string(),
        is_repo: status.is_repo,
        error: status.error.map(|error| logs::redact_sensitive(&error)),
    }))
}
