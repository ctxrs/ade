use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct RepoValidateDestinationReq {
    path: String,
    #[serde(default)]
    must_not_exist: bool,
    #[serde(default)]
    require_empty_if_exists: bool,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct RepoValidateDestinationResp {
    path: String,
}

pub(in crate::api) async fn repo_validate_destination(
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<RepoValidateDestinationReq>,
) -> Result<Json<RepoValidateDestinationResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    repo_validate_destination_impl(req).await
}

pub(in crate::api) async fn repo_validate_destination_get(
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Query(req): Query<RepoValidateDestinationReq>,
) -> Result<Json<RepoValidateDestinationResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    repo_validate_destination_impl(req).await
}

async fn repo_validate_destination_impl(
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

#[derive(Debug, Serialize)]
pub(in crate::api) struct RepoStagingPathResp {
    path: String,
}

/// Returns a unique staging path under data_root/workspaces/staging/<uuid>.
/// Used for disk-isolated clone/new: the daemon manages the path so the wizard
/// doesn't need to ask the user for a host destination.
pub(in crate::api) async fn repo_staging_path(
    mobile_auth: Option<Extension<MobileAuthContext>>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<RepoStagingPathResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let staging_dir = state
        .core
        .data_root
        .join("workspaces")
        .join("staging")
        .join(Uuid::new_v4().to_string());

    tokio::fs::create_dir_all(&staging_dir).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: format!(
                    "failed to create staging dir '{}': {e}",
                    staging_dir.display()
                ),
            }),
        )
    })?;

    let path = tokio::fs::canonicalize(&staging_dir)
        .await
        .unwrap_or(staging_dir)
        .to_string_lossy()
        .to_string();

    Ok(Json(RepoStagingPathResp { path }))
}
