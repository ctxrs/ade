use super::*;
use validation::validate_destination;

#[path = "destination/validation.rs"]
mod validation;

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
    validate_destination(req).await
}

pub(in crate::api) async fn repo_validate_destination_get(
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Query(req): Query<RepoValidateDestinationReq>,
) -> Result<Json<RepoValidateDestinationResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    validate_destination(req).await
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
