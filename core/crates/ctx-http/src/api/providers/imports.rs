use super::*;

pub(super) fn import_result_requires_provider_restart(
    result: &provider_auth_import::ProviderAuthImportResult,
) -> bool {
    // `already_imported` can still mutate active account selection (dedupe/upsert paths),
    // so treat it as auth-affecting to avoid stale runtime credentials.
    matches!(
        result.status.as_str(),
        "imported" | "updated" | "already_imported"
    )
}

pub(crate) async fn list_provider_auth_import_candidates(
    _state: State<Arc<AppState>>,
) -> Result<Json<ProviderAuthImportCandidatesResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let candidates = provider_auth_import::list_provider_auth_import_candidates()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(Json(ProviderAuthImportCandidatesResponse { candidates }))
}

pub(crate) async fn list_provider_auth_import_profiles(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProviderAuthImportProfilesResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let profiles = provider_auth_import::list_provider_auth_profiles(&state.core.data_root)
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
    let mut ids = Vec::new();
    for id in req.candidate_ids {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            ids.push(trimmed.to_string());
        }
    }
    let results =
        provider_auth_import::import_provider_auth_candidates(&state.core.data_root, &ids)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: e.to_string(),
                    }),
                )
            })?;
    let mutated_providers: HashSet<String> = results
        .iter()
        .filter(|result| import_result_requires_provider_restart(result))
        .map(|result| result.provider_id.clone())
        .collect();
    for provider_id in mutated_providers {
        restarts::restart_provider_for_auth_change(
            &state,
            &provider_id,
            &format!("{provider_id} auth updated"),
        )
        .await;
    }
    Ok(Json(ProviderAuthImportResponse { results }))
}
