use std::path::PathBuf;

use axum::http::StatusCode;
use axum::Json;

use crate::api::errors::ApiErrorResp;
use crate::api::repo::{expand_tilde, validate_absolute_path};

use super::RepoInitReq;

pub(super) async fn prepare_repo_init_path(
    req: RepoInitReq,
) -> Result<PathBuf, (StatusCode, Json<ApiErrorResp>)> {
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

    Ok(path)
}
