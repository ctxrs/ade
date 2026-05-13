use super::common::{internal_error, provider_account_mutation_error, unknown_account};
use super::*;

pub(crate) async fn amp_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<AmpAccountsResponse> {
    let registry =
        crate::daemon::providers::ensure_amp_account_registry_from_runtime_auth(state).await?;
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
    crate::daemon::providers::upsert_amp_account(&state, req.label, req.email)
        .await
        .map_err(provider_account_mutation_error)?;
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
        let registry = crate::daemon::providers::load_amp_account_registry(&state)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    crate::daemon::providers::set_active_amp_account(&state, req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    let response = amp_accounts_response(&state)
        .await
        .map_err(internal_error)?;
    Ok(Json(response))
}

pub(crate) async fn delete_amp_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::daemon::providers::remove_amp_account(&state, &id)
        .await
        .map_err(provider_account_mutation_error)?;
    let response = amp_accounts_response(&state)
        .await
        .map_err(internal_error)?;
    Ok(Json(response))
}
