use super::common::{bad_request, internal_error, provider_account_delete_error, unknown_account};
use super::*;

pub(crate) async fn gemini_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<GeminiAccountsResponse> {
    let registry = crate::daemon::providers::load_gemini_account_registry(state).await?;
    Ok(GeminiAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_gemini_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        gemini_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_gemini_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GeminiAccountUpsertReq>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_gemini_account(
        &state.core.data_root,
        req.label,
        req.oauth_creds_json,
        req.google_accounts_json,
        req.email,
    )
    .await
    .map_err(bad_request)?;
    crate::daemon::providers::restart_gemini_providers_for_auth_change(
        &state,
        "gemini auth updated",
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(
        gemini_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_gemini_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GeminiActiveAccountReq>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = crate::daemon::providers::load_gemini_account_registry(&state)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    provider_accounts::set_active_gemini_account(&state.core.data_root, req.account_id)
        .await
        .map_err(bad_request)?;
    crate::daemon::providers::restart_gemini_providers_for_auth_change(
        &state,
        "gemini auth updated",
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(
        gemini_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_gemini_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_gemini_account(&state.core.data_root, &id)
        .await
        .map_err(provider_account_delete_error)?;
    crate::daemon::providers::restart_gemini_providers_for_auth_change(
        &state,
        "gemini auth updated",
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(
        gemini_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}
