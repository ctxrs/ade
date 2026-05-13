use super::*;

pub(crate) async fn list_provider_auth_import_candidates(
    _state: State<Arc<AppState>>,
) -> Result<Json<ProviderAuthImportCandidatesResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let candidates = crate::daemon::providers::list_provider_auth_import_candidates()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    Ok(Json(ProviderAuthImportCandidatesResponse { candidates }))
}

pub(crate) async fn list_provider_auth_import_profiles(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProviderAuthImportProfilesResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let profiles = crate::daemon::providers::list_provider_auth_import_profiles(&state)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(Json(ProviderAuthImportProfilesResponse { profiles }))
}

pub(crate) async fn import_provider_auth_candidates(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ProviderAuthImportReq>,
) -> Result<Json<ProviderAuthImportResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let results =
        crate::daemon::providers::import_provider_auth_candidates(&state, req.candidate_ids)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: e.to_string(),
                    }),
                )
            })?;
    Ok(Json(ProviderAuthImportResponse { results }))
}
