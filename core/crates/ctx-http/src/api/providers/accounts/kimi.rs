use super::common::{bad_request, internal_error, provider_account_delete_error, unknown_account};
use super::*;

pub(crate) async fn kimi_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<KimiAccountsResponse> {
    let registry = provider_accounts::load_kimi_registry(&state.core.data_root).await?;
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
    provider_accounts::add_kimi_account(
        &state.core.data_root,
        req.label,
        req.provider,
        req.credentials_json,
        req.config_toml,
        req.email,
    )
    .await
    .map_err(bad_request)?;
    restarts::restart_kimi_providers_for_auth_change(&state, "kimi auth updated")
        .await
        .map_err(internal_error)?;
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
        let registry = provider_accounts::load_kimi_registry(&state.core.data_root)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    provider_accounts::set_active_kimi_account(&state.core.data_root, req.account_id)
        .await
        .map_err(bad_request)?;
    restarts::restart_kimi_providers_for_auth_change(&state, "kimi auth updated")
        .await
        .map_err(internal_error)?;
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
    provider_accounts::remove_kimi_account(&state.core.data_root, &id)
        .await
        .map_err(provider_account_delete_error)?;
    restarts::restart_kimi_providers_for_auth_change(&state, "kimi auth updated")
        .await
        .map_err(internal_error)?;
    Ok(Json(
        kimi_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}
