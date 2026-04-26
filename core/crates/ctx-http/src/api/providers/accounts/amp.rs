use super::common::{bad_request, internal_error, provider_account_delete_error, unknown_account};
use super::*;

pub(crate) async fn amp_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<AmpAccountsResponse> {
    let registry =
        provider_accounts::ensure_amp_registry_from_runtime_auth(&state.core.data_root).await?;
    Ok(AmpAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_amp_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = amp_accounts_response(&state)
        .await
        .map_err(internal_error)?;
    Ok(Json(response))
}

pub(crate) async fn upsert_amp_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AmpAccountUpsertReq>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::upsert_amp_account(&state.core.data_root, req.label, req.email)
        .await
        .map_err(bad_request)?;
    restarts::restart_amp_providers_for_auth_change(&state, "amp auth updated").await;
    Ok(Json(
        amp_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_amp_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AmpActiveAccountReq>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_amp_registry(&state.core.data_root)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    provider_accounts::set_active_amp_account(&state.core.data_root, req.account_id)
        .await
        .map_err(bad_request)?;
    restarts::restart_amp_providers_for_auth_change(&state, "amp auth updated").await;
    let response = amp_accounts_response(&state)
        .await
        .map_err(internal_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_amp_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_amp_account(&state.core.data_root, &id)
        .await
        .map_err(provider_account_delete_error)?;
    restarts::restart_amp_providers_for_auth_change(&state, "amp auth updated").await;
    let response = amp_accounts_response(&state)
        .await
        .map_err(internal_error)?;
    Ok(Json(response))
}
