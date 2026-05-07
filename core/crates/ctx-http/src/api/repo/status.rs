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
    let expanded = expand_tilde(raw)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;
    validate_absolute_path(&expanded, "path")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;
    let canonical = tokio::fs::canonicalize(&expanded).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("invalid path '{}': {e}", expanded.to_string_lossy()),
            }),
        )
    })?;

    let canonical_str = canonical.to_string_lossy().to_string();
    let driver = match vcs::driver_for_path(&canonical).await {
        Ok(d) => d,
        Err(err) => {
            return Ok(Json(RepoStatusResp {
                canonical_path: canonical_str,
                is_repo: false,
                error: Some(logs::redact_sensitive(&err.to_string())),
            }));
        }
    };
    match driver.assert_repo(&canonical).await {
        Ok(()) => Ok(Json(RepoStatusResp {
            canonical_path: canonical_str,
            is_repo: true,
            error: None,
        })),
        Err(err) => Ok(Json(RepoStatusResp {
            canonical_path: canonical_str,
            is_repo: false,
            error: Some(logs::redact_sensitive(&err.to_string())),
        })),
    }
}
