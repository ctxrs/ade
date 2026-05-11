use std::path::PathBuf;

use super::*;

pub(super) async fn prepare_clone_destination(
    req: &RepoCloneReq,
    repo_url: &str,
) -> Result<PathBuf, (StatusCode, Json<ApiErrorResp>)> {
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
        .or_else(|| derive_repo_name(repo_url))
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

    Ok(dest)
}
