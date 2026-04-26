use super::common::{bad_request, internal_error, provider_account_delete_error, unknown_account};
use super::*;

pub(crate) async fn qwen_accounts_response(
    state: &Arc<AppState>,
) -> anyhow::Result<QwenAccountsResponse> {
    let registry = provider_accounts::load_qwen_registry(&state.core.data_root).await?;
    Ok(QwenAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

pub(crate) async fn list_qwen_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        qwen_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn upsert_qwen_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QwenAccountUpsertReq>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_qwen_account(
        &state.core.data_root,
        req.label,
        req.oauth_creds_json,
        req.email,
    )
    .await
    .map_err(bad_request)?;
    restarts::restart_qwen_providers_for_auth_change(&state, "qwen auth updated")
        .await
        .map_err(internal_error)?;
    Ok(Json(
        qwen_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn set_qwen_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QwenActiveAccountReq>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_qwen_registry(&state.core.data_root)
            .await
            .map_err(internal_error)?;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err(unknown_account());
        }
    }
    provider_accounts::set_active_qwen_account(&state.core.data_root, req.account_id)
        .await
        .map_err(bad_request)?;
    restarts::restart_qwen_providers_for_auth_change(&state, "qwen auth updated")
        .await
        .map_err(internal_error)?;
    Ok(Json(
        qwen_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}

pub(crate) async fn delete_qwen_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_qwen_account(&state.core.data_root, &id)
        .await
        .map_err(provider_account_delete_error)?;
    restarts::restart_qwen_providers_for_auth_change(&state, "qwen auth updated")
        .await
        .map_err(internal_error)?;
    Ok(Json(
        qwen_accounts_response(&state)
            .await
            .map_err(internal_error)?,
    ))
}
