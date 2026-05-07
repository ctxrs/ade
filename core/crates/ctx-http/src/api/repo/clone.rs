use super::*;

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

    let dest_parent = expand_tilde(&req.dest_parent)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    validate_absolute_path(&dest_parent, "dest_parent")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    // Allow cloning into a destination parent that doesn't exist yet by creating it.
    // This keeps the wizard UX simple (users can type a new folder path).
    if !dest_parent.exists() {
        tokio::fs::create_dir_all(&dest_parent).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!(
                        "failed to create dest_parent '{}': {e}",
                        dest_parent.to_string_lossy()
                    ),
                }),
            )
        })?;
    }

    let dest_parent = tokio::fs::canonicalize(&dest_parent).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!(
                    "invalid dest_parent '{}': {}",
                    dest_parent.to_string_lossy(),
                    e
                ),
            }),
        )
    })?;

    let name = req
        .dest_name
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| derive_repo_name(&repo_url))
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "could not derive repo name".to_string(),
            }),
        ))?;

    if let Err(e) = validate_dest_name(&name) {
        return Err((StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })));
    }

    let dest = dest_parent.join(&name);
    if dest.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("destination already exists: {}", dest.display()),
            }),
        ));
    }

    let mut cmd = Command::new("git");
    cmd.arg("clone");
    if let Some(branch) = req
        .branch
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        cmd.arg("--branch").arg(branch).arg("--single-branch");
    }
    cmd.arg("--").arg(&repo_url).arg(&dest);

    let output = cmd.output().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: format!("failed to spawn git: {e}"),
            }),
        )
    })?;
    if !output.status.success() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&format!(
                    "git clone failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )),
            }),
        ));
    }

    let canonical_dest = tokio::fs::canonicalize(&dest).await.unwrap_or(dest);

    Ok(Json(RepoCloneResp {
        path: canonical_dest.to_string_lossy().to_string(),
    }))
}
