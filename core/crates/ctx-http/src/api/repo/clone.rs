use super::*;
use destination::prepare_clone_destination;

#[path = "clone/destination.rs"]
mod destination;

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
    ensure_git_usable()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    let repo_url = req.repo_url.trim().to_string();
    if repo_url.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "repo_url is required".to_string(),
            }),
        ));
    }

    let dest = prepare_clone_destination(&req, &repo_url).await?;
    ctx_workspace_services::repo_onboarding::run_git_clone(
        &repo_url,
        req.branch
            .as_ref()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty()),
        &dest,
    )
    .await
    .map_err(repo_git_command_error_response)?;
    let canonical_dest = ctx_workspace_services::repo_onboarding::canonical_clone_dest(dest).await;

    Ok(Json(RepoCloneResp {
        path: canonical_dest.to_string_lossy().to_string(),
    }))
}
