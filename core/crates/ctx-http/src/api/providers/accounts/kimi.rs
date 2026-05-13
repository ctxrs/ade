use super::common::{internal_error, provider_account_mutation_error, unknown_account};
use super::*;

pub(crate) async fn kimi_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<KimiAccountsResponse> {
    let registry = crate::daemon::providers::load_kimi_account_registry(state).await?;
    Ok(KimiAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_kimi_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        kimi_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_kimi_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<KimiAccountUpsertReq>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::daemon::providers::add_kimi_account(
        &state,
        req.label,
        req.provider,
        req.credentials_json,
        req.config_toml,
        req.email,
    )
    .await
    .map_err(provider_account_mutation_error)?;
    Ok(Json(
        kimi_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_kimi_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<KimiActiveAccountReq>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = crate::daemon::providers::load_kimi_account_registry(&state)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    crate::daemon::providers::set_active_kimi_account(&state, req.account_id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        kimi_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_kimi_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::daemon::providers::remove_kimi_account(&state, &id)
        .await
        .map_err(provider_account_mutation_error)?;
    Ok(Json(
        kimi_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}
