use axum::http::StatusCode;
use axum::Json;

use crate::api::errors::ApiErrorResp;
use crate::api::repo::{expand_tilde, validate_absolute_path};

use super::{RepoValidateDestinationReq, RepoValidateDestinationResp};

pub(super) async fn validate_destination(
    req: RepoValidateDestinationReq,
) -> Result<Json<RepoValidateDestinationResp>, (StatusCode, Json<ApiErrorResp>)> {
    let raw = req.path.trim();
    if raw.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "path is required".to_string(),
            }),
        ));
    }
    let expanded = expand_tilde(raw)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;
    validate_absolute_path(&expanded, "path")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    let metadata = match tokio::fs::metadata(&expanded).await {
        Ok(meta) => Some(meta),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!(
                        "failed to inspect destination '{}': {err}",
                        expanded.display()
                    ),
                }),
            ));
        }
    };

    if let Some(meta) = metadata {
        if !meta.is_dir() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!(
                        "destination exists and is not a directory: {}",
                        expanded.display()
                    ),
                }),
            ));
        }

        if req.must_not_exist {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("destination already exists: {}", expanded.display()),
                }),
            ));
        }

        if req.require_empty_if_exists {
            let mut dir = tokio::fs::read_dir(&expanded).await.map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("failed to read directory '{}': {e}", expanded.display()),
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
                            error: format!(
                                "failed to read directory '{}': {e}",
                                expanded.display()
                            ),
                        }),
                    )
                })?
                .is_some();
            if has_entries {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("destination is not empty: {}", expanded.display()),
                    }),
                ));
            }
        }
    }

    let resolved = tokio::fs::canonicalize(&expanded).await.unwrap_or(expanded);
    Ok(Json(RepoValidateDestinationResp {
        path: resolved.to_string_lossy().to_string(),
    }))
}
