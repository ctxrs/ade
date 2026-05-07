use super::*;

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
    ensure_git_usable()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    let raw = req.path.trim();
    if raw.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "path is required".to_string(),
            }),
        ));
    }

    let path = expand_tilde(raw)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    validate_absolute_path(&path, "path")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    if path.exists() && !req.allow_existing {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("destination already exists: {}", path.display()),
            }),
        ));
    }
    tokio::fs::create_dir_all(&path).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("failed to create directory '{}': {e}", path.display()),
            }),
        )
    })?;

    // By default we refuse to init into a non-empty directory.
    // Import onboarding can opt in with allow_non_empty=true after explicit user confirmation.
    let mut dir = tokio::fs::read_dir(&path).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("failed to read directory '{}': {e}", path.display()),
            }),
        )
    })?;
    let has_entries = dir
        .next_entry()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("failed to read directory '{}': {e}", path.display()),
                }),
            )
        })?
        .is_some();
    if has_entries && !req.allow_non_empty {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("destination is not empty: {}", path.display()),
            }),
        ));
    }

    let output = Command::new("git")
        .arg("init")
        .arg("--")
        .arg(&path)
        .output()
        .await
        .map_err(|e| {
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
                    "git init failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )),
            }),
        ));
    }

    // Worktrees require a base commit to diff against. `git init` alone yields a repo with no
    // commits, which breaks the out-of-the-box wizard path ("New repo").
    //
    // We create an empty initial commit using inline identity overrides, so we don't depend on the
    // user's global git config (user.name/user.email).
    let output = Command::new("git")
        .arg("-C")
        .arg(&path)
        .arg("-c")
        .arg("user.name=ctx")
        .arg("-c")
        .arg("user.email=ctx@localhost")
        .arg("commit")
        .arg("--allow-empty")
        .arg("-m")
        .arg("Initial commit")
        .output()
        .await
        .map_err(|e| {
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
                    "git commit failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )),
            }),
        ));
    }

    let canonical = tokio::fs::canonicalize(&path).await.unwrap_or(path);

    Ok(Json(RepoInitResp {
        path: canonical.to_string_lossy().to_string(),
    }))
}
